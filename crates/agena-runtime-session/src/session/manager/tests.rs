//! Manager-level regression tests expressed exclusively through the v2
//! facade/parts model. Storage invariants, concurrency, recovery, retry,
//! usage, and JSONL have their exhaustive engine/facade suites in
//! `agena-storage` and `agena-storage-sqlite`; these tests prove the execution
//! manager's adapter preserves that model at its boundary.

use std::{collections::BTreeMap, collections::HashMap, sync::Arc};

use agena_domain::{
    AssistantReasoningField, AssistantReplyId, ComposerDocument, ComposerNode, ModelId, ModelRef,
    ProviderId, Role, StructuredObject, TimeRange, ToolInvocation, TurnId, UserInputOption,
    UserInputQuestion, UserInputReply, UserInputReplyKind, UserInputSource,
};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_provider::{
    CompletionFinishReason, CompletionInputPart, CompletionRequest, CompletionResponse,
    CompletionStreamEvent, CompletionUsage,
};
use agena_storage::store::{
    NewPart, PartDelta, PartRole, PartState, PartVisibility, PersistenceEngine, SessionChange,
    SessionState, SubmitOutcome,
};
use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement};

use super::{
    ExecutionConversationTarget, SessionManager, SessionRunRequest, SessionRunTermination,
    SessionSubtaskRequest, SessionUserRunRequest, merge_system_prompts,
};
use crate::provider::{ModelRuntime, ProviderError};
use crate::session::manager::replies::{operation_from_part, operation_id_from_part};
use crate::session::manager::runs::run_visible_text_lossy;
use crate::session::store::{
    OPERATION_ID_METADATA_KEY, ProcessorPartIdAllocator, new_part_from_content, parts_into_runs,
    run_marker_content, text_content, tool_call_from_operation, typed_content_from_value,
    typed_content_to_value,
};
use crate::{
    ContextGovernor, RuntimeSessionManagerConfig, SessionExecutionReplyRequest,
    authorization::ExecutionPrincipal,
    permission::{PermissionPolicy, ToolPermissionPolicy},
    provider::ProviderRegistry,
    session::{Session, SessionProcessor},
    tool::ToolExecutor,
};
use agena_runtime_contracts::part_content::{
    SystemNotificationContent, TypedContent, operation_from_tool_call,
};

async fn test_manager() -> SessionManager {
    test_manager_with_database().await.0
}

async fn test_manager_with_database() -> (SessionManager, DatabaseConnection) {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: Vec::new(),
        config: PluginsConfig::default(),
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build empty plugin host");
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        Arc::clone(&plugins),
        None,
        None,
        None,
    );
    let provider_registry = Arc::new(ProviderRegistry::new());
    let context_governor = ContextGovernor::new(agena_domain::ContextPolicy::default());
    let processor = SessionProcessor::new(plugins);
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("open v2 test database");
    initialize(&database).await;
    let manager = SessionManager::new(
        database.clone(),
        provider_registry,
        context_governor,
        processor,
        executor,
        RuntimeSessionManagerConfig::default(),
    );
    (manager, database)
}

async fn initialize(database: &DatabaseConnection) {
    agena_storage_sqlite::initialize_schema(database)
        .await
        .expect("initialize fresh v2 schema");
}

async fn create(manager: &SessionManager, title: &str) -> Session {
    manager
        .create_session(agena_runtime::SessionCreateRequest {
            title: title.to_owned(),
            parent_session_id: None,
        })
        .await
        .expect("create session")
}

async fn create_with_model(
    manager: &SessionManager,
    title: &str,
    provider_id: &str,
    model_id: &str,
) -> Session {
    let session = create(manager, title).await;
    manager
        .set_session_model_override(session.id, ModelRef::new(provider_id, model_id))
        .await
        .expect("persist explicit test model selection")
}

#[tokio::test]
async fn cancellation_force_aborts_unresponsive_operation_and_releases_registry() {
    let manager = test_manager().await;
    let session = create(&manager, "unresponsive cancellation").await;
    let session_id = session.id;
    let owner = manager.background_handle();
    let execution = tokio::spawn(async move {
        owner
            .execute_registered(
                session_id,
                agena_domain::ExecutionSource::User,
                ExecutionConversationTarget::NewTurn,
                "unresponsive cancellation fixture",
                |_manager, _control, _steer_rx| async move {
                    std::future::pending::<Result<(), crate::AppError>>().await
                },
            )
            .await
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !manager.execution_registry.is_active(session_id).await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("execution registers");
    manager
        .cancel_active_execution_with_outcome(session_id)
        .await
        .expect("request cancellation");

    let error = tokio::time::timeout(std::time::Duration::from_secs(2), execution)
        .await
        .expect("lifecycle owner finishes within cancellation budget")
        .expect("request task joins")
        .expect_err("unresponsive operation is cancelled");
    assert!(matches!(error, crate::AppError::Cancelled));
    assert!(!manager.execution_registry.is_active(session_id).await);
}

#[tokio::test]
async fn cancellation_suppresses_queued_background_notification_wakes() {
    let manager = test_manager().await;
    let session = create(&manager, "cancel queued delivery").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .expect("start launch run");
    install_test_background_operation(
        &manager,
        session.id,
        run_id,
        agena_storage::store::BackgroundOperationKind::Shell,
        "proc_cancel_queued_delivery",
    )
    .await;
    let operation = manager
        .store
        .background_operation_by_external_id(
            agena_storage::store::BackgroundOperationKind::Shell,
            "proc_cancel_queued_delivery",
        )
        .await
        .expect("load background operation")
        .expect("background operation exists");
    let notification = SystemNotificationContent {
        operation_id: "proc_cancel_queued_delivery".to_owned(),
        operation_kind: "shell".to_owned(),
        status: "completed".to_owned(),
        summary: "exit 0".to_owned(),
        body: "exit 0".to_owned(),
        ..Default::default()
    };
    manager
        .store
        .record_background_event(agena_storage::store::BackgroundEventRequest {
            operation_id: operation.operation_id,
            event_key: "terminal".to_owned(),
            event_seq: None,
            next_phase: Some(agena_storage::store::BackgroundOperationPhase::Completed),
            outcome: Some(serde_json::json!({"text": "exit 0"})),
            failure: None,
            notification: new_part_from_content(
                "system_notification",
                PartRole::Assistant,
                &TypedContent::SystemNotification(notification),
                PartState::Completed,
            )
            .expect("build notification"),
        })
        .await
        .expect("enqueue notification delivery");
    assert_eq!(
        manager
            .store
            .pending_background_deliveries(16)
            .await
            .expect("read pending delivery")
            .len(),
        1
    );

    manager
        .cancel_active_execution_with_outcome(session.id)
        .await
        .expect("cancel session and queued wakes");
    assert!(
        manager
            .store
            .pending_background_deliveries(16)
            .await
            .expect("read deliveries after cancellation")
            .is_empty(),
        "cancel must not leave a background wake that can relaunch the session"
    );
}

#[tokio::test]
async fn cancelling_an_unanswered_user_turn_withdraws_it_and_restores_the_document() {
    let manager = test_manager().await;
    let session = create(&manager, "restore cancelled input").await;
    let document = ComposerDocument(vec![ComposerNode::Text {
        text: "fix this typo".to_owned(),
    }]);
    let expected_document = document.clone();
    let session_id = session.id;
    manager
        .start_registered_with_restore(
            session_id,
            agena_domain::ExecutionSource::User,
            ExecutionConversationTarget::NewTurn,
            "unanswered user cancellation fixture",
            Some(document),
            Some("restore-cancel-key".to_owned()),
            move |manager, control, _steer_rx| async move {
                let part = new_part_from_content(
                    "text",
                    PartRole::User,
                    &TypedContent::Text(text_content("fix this typo")),
                    PartState::Completed,
                )?;
                let execution_id = control.execution_id().to_string();
                let submitted = manager
                    .store
                    .submit_user_run_for_execution(
                        session_id,
                        vec![part],
                        Some("restore-cancel-key".to_owned()),
                        &execution_id,
                    )
                    .await?;
                control.set_user_run(submitted.run_id, submitted.created);
                std::future::pending::<Result<(), crate::AppError>>().await
            },
        )
        .await
        .expect("accept unanswered user execution");

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !manager.execution_registry.is_active(session_id).await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("execution registers");

    let outcome = manager
        .cancel_active_execution_with_outcome(session_id)
        .await
        .expect("cancel unanswered user execution");
    assert_eq!(
        outcome.result,
        agena_domain::CancellationResult::CancellationRequested
    );
    assert_eq!(outcome.restored_user_message, Some(expected_document));
    assert!(outcome.restored_user_run_id.is_some());
    assert!(
        manager
            .store
            .load_session(session_id)
            .await
            .expect("load after cancellation")
            .parts()
            .is_empty()
    );
}

async fn append_message(
    manager: &SessionManager,
    mut session: Session,
    role: Role,
    contents: Vec<TypedContent>,
) -> Session {
    let part_role = match role {
        Role::User => PartRole::User,
        Role::Assistant => PartRole::Assistant,
        Role::System => PartRole::System,
        Role::Tool => PartRole::Tool,
    };
    let new_parts = contents
        .iter()
        .map(|content| new_part_from_content("text", part_role, content, PartState::Completed))
        .collect::<Result<Vec<_>, _>>()
        .expect("build message parts");
    // A user append is one `user_send` run (marker + parts); any other role
    // appends content parts under a `continue` assistant run marker.
    let outcome = if role == Role::User {
        manager
            .store
            .submit_user_run(session.id, new_parts, None)
            .await
            .expect("submit user message through facade")
    } else {
        let run_id = manager
            .store
            .start_run(
                session.id,
                "continue",
                run_marker_content("continue", None, None, None, None),
            )
            .await
            .expect("start assistant run");
        let created = manager
            .store
            .append_parts(session.id, run_id, new_parts)
            .await
            .expect("append assistant parts");
        SubmitOutcome {
            run_id,
            created: true,
            parts: created,
        }
    };
    let mut projected = session.parts().to_vec();
    projected.extend(outcome.parts);
    session.install_projected_parts(projected);
    session
}

#[test]
fn system_prompt_merge_is_idempotent() {
    assert_eq!(
        merge_system_prompts(Some("identity"), Some("identity\n\ncustom")),
        Some("identity\n\ncustom".to_owned())
    );
    assert_eq!(
        merge_system_prompts(Some("identity"), Some("custom")),
        Some("identity\n\ncustom".to_owned())
    );
}

#[tokio::test]
async fn create_and_reload_use_the_sealed_facade() {
    let manager = test_manager().await;
    let created = create(&manager, "parts session").await;
    let reloaded = manager
        .get_session(created.id)
        .await
        .expect("reload session");
    assert_eq!(reloaded.id, created.id);
    assert_eq!(reloaded.title, "parts session");
    assert!(reloaded.parts().is_empty());

    let view = manager
        .session_store()
        .load(created.id)
        .await
        .expect("load facade view");
    assert_eq!(view.meta.id, created.id);
    assert!(view.parts.is_empty());
}

#[tokio::test]
async fn session_workspace_id_scopes_tools_to_the_project_root() {
    let manager = test_manager().await;
    let project = tempfile::tempdir().expect("create project workspace");
    let snapshot = tempfile::tempdir().expect("create snapshot workspace");
    let project_path = std::fs::canonicalize(project.path()).expect("canonicalize project");
    let snapshot_path = std::fs::canonicalize(snapshot.path()).expect("canonicalize snapshot");

    let workspace = manager
        .workspace_repository
        .create(project_path.to_string_lossy().into_owned())
        .await
        .expect("create secondary workspace");
    let session = manager
        .store
        .create_session(
            workspace.id,
            None,
            agena_domain::SessionRelationKind::Root,
            None,
            "project-root session".to_owned(),
            None,
            None,
        )
        .await
        .expect("create session in secondary workspace");

    let server_root = manager
        .execution_state()
        .tool_executor
        .workspace_root()
        .to_path_buf();
    assert_ne!(
        server_root, project_path,
        "the fixture must separate the server process workspace from the session project"
    );

    let mut loaded = manager
        .get_session(session.id)
        .await
        .expect("load workspace-bound session");
    assert_eq!(
        loaded.runtime.effective_workspace_root(),
        Some(project_path.as_path())
    );
    let project_executor = manager
        .execution_state()
        .tool_executor
        .for_session_context_async(&loaded.runtime.execution)
        .await;
    assert_eq!(
        project_executor.workspace_root(),
        project_path.as_path(),
        "tools for a normal root session must run in the session's project, not the server root"
    );

    loaded
        .runtime
        .set_effective_workspace_root(Some(snapshot_path.clone()));
    loaded = manager
        .store
        .persist_execution_config(loaded)
        .await
        .expect("persist snapshot override");
    let mut reloaded = manager
        .get_session(loaded.id)
        .await
        .expect("reload snapshot-bound session");
    assert_eq!(
        reloaded.runtime.effective_workspace_root(),
        Some(snapshot_path.as_path()),
        "a snapshot/worktree override takes precedence over the project root"
    );

    reloaded.runtime.set_effective_workspace_root(None);
    reloaded = manager
        .store
        .persist_execution_config(reloaded)
        .await
        .expect("clear snapshot override");
    let reloaded = manager
        .get_session(reloaded.id)
        .await
        .expect("reload project after snapshot exit");
    assert_eq!(
        reloaded.runtime.effective_workspace_root(),
        Some(project_path.as_path()),
        "exiting a snapshot must return tools to the owning project instead of the server root"
    );
    let restored_executor = manager
        .execution_state()
        .tool_executor
        .for_session_context_async(&reloaded.runtime.execution)
        .await;
    assert_eq!(restored_executor.workspace_root(), project_path.as_path());
}

#[tokio::test]
async fn session_model_selection_survives_a_store_reload() {
    let manager = test_manager().await;
    let session = create(&manager, "model selection persistence").await;
    let updated = manager
        .update_session_selection(
            session.id,
            agena_runtime::SessionRunOptions {
                model: ModelRef::new("acme", "acme-fast"),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
            },
        )
        .await
        .expect("update session selection");
    assert_eq!(
        manager
            .model_from_session_selection(&updated)
            .expect("updated selection parses")
            .map(|model| model.model_id.to_string()),
        Some("acme-fast".to_owned()),
        "the returned session carries the new model"
    );

    // The bug: the selection was only mutated in memory, so a reload from the
    // store dropped it and the next turn lost the selected model. It must
    // be persisted to `sessions.config_json` and restored on load.
    let reloaded = manager
        .get_session(session.id)
        .await
        .expect("reload session after selection update");
    assert_eq!(
        manager
            .model_from_session_selection(&reloaded)
            .expect("reloaded selection parses")
            .map(|model| model.model_id.to_string()),
        Some("acme-fast".to_owned()),
        "the selected model must be restored from the store after a reload"
    );
}

#[tokio::test]
async fn messages_are_run_markers_plus_ordered_parts() {
    let manager = test_manager().await;
    let session = create(&manager, "ordered parts").await;
    let session = append_message(
        &manager,
        session,
        Role::User,
        vec![
            TypedContent::Text(text_content("first")),
            TypedContent::Text(text_content("second")),
        ],
    )
    .await;
    let session = append_message(
        &manager,
        session,
        Role::Assistant,
        vec![TypedContent::Text(text_content("answer"))],
    )
    .await;

    let view = manager
        .session_store()
        .load(session.id)
        .await
        .expect("load ordered parts");
    assert_eq!(view.parts.len(), 5, "two markers plus three content parts");
    assert_eq!(
        view.parts.iter().filter(|part| part.kind == "run").count(),
        2
    );
    assert!(
        view.parts
            .windows(2)
            .all(|pair| (pair[0].created_at_ms, pair[0].part_id)
                <= (pair[1].created_at_ms, pair[1].part_id))
    );
    assert!(view.parts.iter().all(|part| {
        part.kind == "run"
            || part.run_id.is_some_and(|run_id| {
                view.parts
                    .iter()
                    .any(|candidate| candidate.part_id == run_id && candidate.kind == "run")
            })
    }));
}

#[tokio::test]
async fn one_checkpoint_updates_only_the_named_part_revision() {
    let manager = test_manager().await;
    let session = create(&manager, "checkpoint").await;
    let session = append_message(
        &manager,
        session,
        Role::Assistant,
        vec![
            TypedContent::Text(text_content("left")),
            TypedContent::Text(text_content("right")),
        ],
    )
    .await;
    let before = manager
        .session_store()
        .load(session.id)
        .await
        .expect("load before checkpoint");
    let content_parts = before
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| (part.part_id, part.revision))
        .collect::<Vec<_>>();
    assert_eq!(content_parts.len(), 2);

    let mut aggregate = manager
        .get_session(session.id)
        .await
        .expect("load aggregate");
    let text_indices = aggregate
        .parts()
        .iter()
        .enumerate()
        .filter(|(_, part)| part.kind == "text")
        .map(|(index, part)| (index, part.part_id))
        .collect::<Vec<_>>();
    let (changed_index, changed_part_id) = text_indices[1];
    {
        let changed = aggregate
            .part_mut(&crate::session::model::SessionPartRef {
                part_index: changed_index,
                part_id: changed_part_id,
            })
            .expect("changed part");
        changed.content =
            typed_content_to_value(&TypedContent::Text(text_content("right updated")))
                .expect("part content is JSON serializable");
    }
    manager
        .persist_session_changes(
            aggregate,
            vec![changed_part_id],
            None,
            manager.execution_state(),
        )
        .await
        .expect("persist exact checkpoint");

    let after = manager
        .session_store()
        .load(session.id)
        .await
        .expect("load after checkpoint");
    let unchanged = after
        .parts
        .iter()
        .find(|part| part.part_id == content_parts[0].0)
        .expect("unchanged part");
    let changed = after
        .parts
        .iter()
        .find(|part| part.part_id == changed_part_id)
        .expect("changed part");
    assert_eq!(unchanged.revision, content_parts[0].1);
    assert!(changed.revision > content_parts[1].1);
    assert_eq!(changed.content["text"], "right updated");
}

#[tokio::test]
async fn fork_renders_the_shared_prefix_without_copying_parts() {
    let manager = test_manager().await;
    let session = create(&manager, "fork source").await;
    let session = append_message(
        &manager,
        session,
        Role::User,
        vec![TypedContent::Text(text_content("shared prompt"))],
    )
    .await;
    let source = manager
        .session_store()
        .load(session.id)
        .await
        .expect("load source");
    let cutoff = source.parts.last().expect("source tail").part_id;
    let child_id = manager
        .store
        .fork(session.id, cutoff, "fork child".to_owned())
        .await
        .expect("fork through facade");
    let child = manager
        .session_store()
        .load(child_id)
        .await
        .expect("load fork");
    assert_eq!(
        child
            .parts
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>(),
        source
            .parts
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>()
    );
    let child_session = manager
        .get_session(child_id)
        .await
        .expect("project fork transcript");
    let child_runs = parts_into_runs(child_session.parts());
    assert_eq!(run_visible_text_lossy(&child_runs[0]), "shared prompt");
}

#[tokio::test]
async fn default_manager_fork_includes_the_complete_last_message() {
    let manager = test_manager().await;
    let session = create(&manager, "default fork source").await;
    let session = append_message(
        &manager,
        session,
        Role::User,
        vec![
            TypedContent::Text(text_content("shared first")),
            TypedContent::Text(text_content("shared second")),
        ],
    )
    .await;

    let child = manager
        .fork_session(agena_runtime::SessionForkRequest {
            session_id: session.id,
            at_message_id: None,
            title: Some("default fork child".to_owned()),
            expected_version: None,
        })
        .await
        .expect("fork complete history through manager");

    let child_runs = parts_into_runs(child.parts());
    assert_eq!(child_runs.len(), 1);
    assert_eq!(
        run_visible_text_lossy(&child_runs[0]),
        "shared first\nshared second"
    );
    let source_view = manager
        .session_store()
        .load(session.id)
        .await
        .expect("load source parts");
    let child_view = manager
        .session_store()
        .load(child.id)
        .await
        .expect("load child parts");
    assert_eq!(
        child_view
            .parts
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>(),
        source_view
            .parts
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>(),
        "the default message cutoff shares the marker and every content part"
    );

    let explicit = manager
        .fork_session(agena_runtime::SessionForkRequest {
            session_id: session.id,
            at_message_id: session
                .parts()
                .iter()
                .find(|part| part.is_run_marker())
                .map(|part| part.part_id),
            title: Some("explicit marker fork child".to_owned()),
            expected_version: None,
        })
        .await
        .expect("fork at an explicit message marker");
    let explicit_runs = parts_into_runs(explicit.parts());
    assert_eq!(explicit_runs.len(), 1);
    assert_eq!(
        run_visible_text_lossy(&explicit_runs[0]),
        "shared first\nshared second",
        "an explicit message marker resolves to that message's inclusive tail"
    );
}

#[tokio::test]
async fn open_session_preserves_a_run_paused_for_user_input_without_a_lease() {
    let (manager, database) = test_manager_with_database().await;
    let session = create(&manager, "awaiting user").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            serde_json::json!({"run_kind": "continue"}),
        )
        .await
        .expect("start in-flight run");
    let mut operation = agena_runtime_contracts::part::OperationPart::pending(
        1,
        ToolInvocation::new("interaction.ask", StructuredObject::default()),
        TimeRange::default(),
    );
    operation
        .user_input
        .push_pending(agena_domain::UserInputRequest {
            request_id: "ask-1".to_owned(),
            session_id: Some(session.id),
            title: "Choose a path".to_owned(),
            body_markdown: String::new(),
            kind: "ask_user".to_owned().into(),
            source: UserInputSource::Plugin,
            auto_resolution_ms: None,
            presented_at: None,
            questions: Vec::new(),
            created_at: chrono::Utc::now(),
        });
    manager
        .store
        .append_parts(
            session.id,
            run_id,
            vec![NewPart::pending(
                "tool_call",
                PartRole::Assistant,
                tool_call_from_operation(&operation).as_value(),
            )],
        )
        .await
        .expect("append pending tool call");

    // Fault injection through the persistence engine: an open session must
    // derive AwaitingInteraction even when the paused run has no lease. Production
    // manager code itself continues to access chat data only through
    // SessionStore.
    let engine = agena_storage_sqlite::SqliteEngine::new(Arc::new(database));
    assert!(
        engine
            .release_lease(session.id, manager.store.owner_id.as_str())
            .await
            .expect("release paused run lease")
    );
    let facade = manager.session_store();
    let before = facade
        .session_state(session.id)
        .await
        .expect("derive awaiting state without a lease");
    assert_eq!(before.state, SessionState::AwaitingInteraction);

    // Exercise the manager's lazy reconciliation-on-open path. If its
    // AwaitingInteraction guard regresses, this call terminalizes the paused run.
    let opened = manager
        .get_session(session.id)
        .await
        .expect("open paused session");
    assert_eq!(opened.id, session.id);

    let after = facade.load(session.id).await.expect("reload paused parts");
    let marker = after
        .parts
        .iter()
        .find(|part| part.part_id == run_id)
        .expect("run marker remains");
    let tool = after
        .parts
        .iter()
        .find(|part| part.kind == "tool_call")
        .expect("tool call remains");
    assert!(marker.state.is_in_flight(), "paused run is not aborted");
    assert_eq!(tool.state, PartState::Pending);
    assert_eq!(
        facade
            .session_state(session.id)
            .await
            .expect("derive state after open")
            .state,
        SessionState::AwaitingInteraction
    );
}

#[tokio::test]
async fn open_session_leaves_another_process_fresh_run_intact() {
    let manager = test_manager().await;
    let session = create(&manager, "fresh run").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            serde_json::json!({"run_kind": "continue"}),
        )
        .await
        .expect("start fresh run");

    manager
        .get_session(session.id)
        .await
        .expect("open session with fresh lease");

    let presentation = manager
        .session_store()
        .session_state(session.id)
        .await
        .expect("derive running state after open");
    assert_eq!(presentation.state, SessionState::Running);
    assert_eq!(presentation.active_run_id, Some(run_id));
    let view = manager
        .session_store()
        .load(session.id)
        .await
        .expect("load live run");
    assert!(
        view.parts
            .iter()
            .find(|part| part.part_id == run_id)
            .expect("run marker")
            .state
            .is_in_flight(),
        "open must not reconcile a fresh run"
    );
}

#[tokio::test]
async fn manager_jsonl_round_trip_restores_parts_as_an_independent_root() {
    let manager = test_manager().await;
    let session = create(&manager, "export source").await;
    let session = append_message(
        &manager,
        session,
        Role::User,
        vec![TypedContent::Text(text_content("round trip"))],
    )
    .await;
    let bundle = manager
        .export_session_jsonl(session.id)
        .await
        .expect("export JSONL");
    let imported = manager
        .import_session_jsonl(&bundle)
        .await
        .expect("import JSONL");
    assert_ne!(imported.id, session.id);
    assert!(imported.parent_id.is_none());
    let imported_runs = parts_into_runs(imported.parts());
    assert_eq!(imported_runs.len(), 1);
    assert_eq!(run_visible_text_lossy(&imported_runs[0]), "round trip");
}

#[tokio::test]
async fn query_projection_is_derived_from_persisted_parts() {
    let manager = test_manager().await;
    let session = create(&manager, "query projection").await;
    let session = append_message(
        &manager,
        session,
        Role::User,
        vec![TypedContent::Text(text_content("query me"))],
    )
    .await;
    let projected = manager
        .list_projected_runs(session.id)
        .await
        .expect("list projected messages");
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].parts.len(), 1);
    assert_eq!(
        projected[0].parts[0]
            .content
            .as_ref()
            .and_then(|value| value.get("text"))
            .and_then(serde_json::Value::as_str),
        Some("query me")
    );
}

#[tokio::test]
async fn projection_preserves_precise_part_kind() {
    let manager = test_manager().await;
    let session = create(&manager, "projection kind").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            serde_json::json!({"run_kind": "continue"}),
        )
        .await
        .expect("start assistant run");
    manager
        .store
        .append_parts(
            session.id,
            run_id,
            vec![
                NewPart {
                    kind: "think".to_owned(),
                    role: PartRole::Assistant,
                    content: serde_json::json!({"summary": ["thinking…"]}),
                    summary: None,
                    visibility: PartVisibility::Both,
                    parent_part_id: None,
                    state: PartState::Completed,
                },
                NewPart {
                    kind: "tool_call".to_owned(),
                    role: PartRole::Assistant,
                    content: serde_json::json!({
                        "name": "tools_search",
                        "input": {"query": "x"},
                        "call_id": 1,
                        "state": "completed",
                        "output": {"text": "search complete", "truncated": false},
                        "lifecycle": {"start_ms": 1, "end_ms": 2}
                    }),
                    summary: None,
                    visibility: PartVisibility::Both,
                    parent_part_id: None,
                    state: PartState::Completed,
                },
            ],
        )
        .await
        .expect("append assistant parts");

    let projected = manager
        .list_projected_runs(session.id)
        .await
        .expect("list projected runs");
    let assistant_run = projected
        .iter()
        .find(|run| run.role == Role::Assistant)
        .expect("assistant run");
    let kinds = assistant_run
        .parts
        .iter()
        .map(|part| part.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        ["think", "tool_call"],
        "projection must preserve the precise v2 part kind, not collapse to \"activity\""
    );
}

#[tokio::test]
async fn derived_state_tracks_run_marker_and_lease() {
    let manager = test_manager().await;
    let session = create(&manager, "derived state").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            serde_json::json!({"run_kind":"continue"}),
        )
        .await
        .expect("start run");
    let running = manager
        .session_store()
        .session_state(session.id)
        .await
        .expect("derive running state");
    assert_eq!(running.state, SessionState::Running);
    manager
        .store
        .complete_run(
            session.id,
            run_id,
            agena_storage::store::RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
        )
        .await
        .expect("complete run");
    let ready = manager
        .session_store()
        .session_state(session.id)
        .await
        .expect("derive ready state");
    assert_eq!(ready.state, SessionState::Ready);
}

/// Fake provider whose `complete_stream` emits a controlled token stream, so
/// the integration test can drive exactly the deltas a real model would send.
struct FakeProvider {
    provider_id: &'static str,
    model: ModelId,
    deltas: Vec<String>,
    thinking_deltas: Vec<String>,
    finish_reason: Option<CompletionFinishReason>,
}

#[derive(Clone, Copy)]
enum ProviderNativeFixtureMode {
    CompletionOmitsId,
    StartedOnly,
}

struct ProviderNativeFixtureProvider {
    model: ModelId,
    mode: ProviderNativeFixtureMode,
}

struct StartupFailureProvider {
    model: ModelId,
}

struct HangingProvider {
    model: ModelId,
}

#[async_trait::async_trait]
impl ModelRuntime for HangingProvider {
    fn id(&self) -> &str {
        "hanging"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        std::future::pending().await
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        Ok(Box::pin(futures_util::stream::pending()))
    }
}

#[async_trait::async_trait]
impl ModelRuntime for StartupFailureProvider {
    fn id(&self) -> &str {
        "startup-failure"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        Err(ProviderError::Provider(
            "fixture upstream rejected request".to_owned(),
        ))
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        Err(ProviderError::Provider(
            "fixture upstream rejected request".to_owned(),
        ))
    }
}

#[async_trait::async_trait]
impl ModelRuntime for FakeProvider {
    fn id(&self) -> &str {
        self.provider_id
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.provider_id),
            model: self.model.clone(),
            text: self.deltas.concat(),
            reasoning_text: None,
            finish_reason: self.finish_reason.clone(),
            tool_calls: Vec::new(),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
        })
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        let mut events: Vec<Result<CompletionStreamEvent, ProviderError>> = Vec::new();
        for delta in &self.thinking_deltas {
            events.push(Ok(CompletionStreamEvent::ThinkingDelta {
                provider_id: ProviderId::new(self.provider_id),
                model: self.model.clone(),
                delta: delta.clone(),
            }));
        }
        for delta in &self.deltas {
            events.push(Ok(CompletionStreamEvent::TextDelta {
                provider_id: ProviderId::new(self.provider_id),
                model: self.model.clone(),
                delta: delta.clone(),
            }));
        }
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id: ProviderId::new(self.provider_id),
            model: self.model.clone(),
            finish_reason: self.finish_reason.clone(),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
            end_turn: None,
        }));
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

#[async_trait::async_trait]
impl ModelRuntime for ProviderNativeFixtureProvider {
    fn id(&self) -> &str {
        "provider-native-fixture"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
            text: String::new(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
        })
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        let provider_id = ProviderId::new(self.id());
        let model = self.model.clone();
        let invocation = ToolInvocation::new(
            "web_search",
            StructuredObject::try_from(serde_json::json!({"query": "fixture"}))
                .expect("structured provider-native input"),
        );
        let mut events = vec![Ok(CompletionStreamEvent::ProviderNativeToolCallStarted {
            provider_id: provider_id.clone(),
            model: model.clone(),
            stream_key: "idx:0".to_owned(),
            id: Some("ws_fixture_1".to_owned()),
            invocation: invocation.clone(),
            title: "Hosted search".to_owned(),
            raw: Some(serde_json::json!({
                "id": "ws_fixture_1",
                "status": "in_progress"
            })),
        })];
        if matches!(self.mode, ProviderNativeFixtureMode::CompletionOmitsId) {
            // A redundant progress/start fragment may omit fields or carry
            // structurally empty replacements. It must not erase the hosted
            // call's identity, title, or raw provider context.
            events.push(Ok(CompletionStreamEvent::ProviderNativeToolCallStarted {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key: "idx:7".to_owned(),
                id: Some("ws_fixture_1".to_owned()),
                invocation: invocation.clone(),
                title: "   ".to_owned(),
                raw: Some(serde_json::json!({
                    "id": "   ",
                    "status": null,
                    "result": "done"
                })),
            }));
            let completed_event = CompletionStreamEvent::ProviderNativeToolCallCompleted {
                provider_id: provider_id.clone(),
                model: model.clone(),
                stream_key: "idx:7".to_owned(),
                id: None,
                // The terminal snapshot omits the query carried by the
                // started item. It must not erase the richer invocation.
                invocation: ToolInvocation::new("web_search", StructuredObject::default()),
                title: String::new(),
                summary: "1 result".to_owned(),
                output_text: "fixture result".to_owned(),
                blocks: vec![agena_provider::ProviderNativeToolOutputBlock::Text {
                    text: "fixture result".to_owned(),
                }],
                details: agena_domain::ToolOutput::default(),
                raw: None,
            };
            events.push(Ok(completed_event.clone()));
            events.push(Ok(completed_event));
        }
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id,
            model,
            finish_reason: Some(CompletionFinishReason::Stop),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
            end_turn: None,
        }));
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// Manager wired to a [`SessionProcessor`] whose registry holds `provider`.
async fn manager_with_provider(provider: Arc<dyn ModelRuntime>) -> SessionManager {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: Vec::new(),
        config: PluginsConfig::default(),
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build empty plugin host");
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        Arc::clone(&plugins),
        None,
        None,
        None,
    );
    let mut registry = ProviderRegistry::new();
    registry.register_arc(provider);
    let provider_registry = Arc::new(registry);
    let context_governor = ContextGovernor::new(agena_domain::ContextPolicy::default());
    let processor = SessionProcessor::new(plugins);
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("open v2 test database");
    initialize(&database).await;
    SessionManager::new(
        database,
        provider_registry,
        context_governor,
        processor,
        executor,
        RuntimeSessionManagerConfig::default(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opening_a_new_subtask_at_running_publication_cannot_mark_it_interrupted() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["subtask completed".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let parent = create_with_model(
        &manager,
        "subtask launch reconciliation race",
        "fake",
        "fake-model",
    )
    .await;
    let observer_manager = manager.background_handle();
    let (observed_tx, observed_rx) = std::sync::mpsc::sync_channel(1);
    let observed_tx = Arc::new(std::sync::Mutex::new(Some(observed_tx)));

    // Force the exact production race: a session-tree consumer opens the
    // child synchronously from the notification that first publishes its
    // Running metadata. This is before run_subtask reaches
    // execute_registered. Without the live-launch reconciliation claim,
    // get_session classifies the brand-new child as a restart orphan and
    // writes Interrupted four milliseconds before execution starts.
    let _subscription = manager
        .session_store()
        .subscribe_all(Arc::new(move |change| {
            let SessionChange::SessionMetaUpdated { meta, .. } = change else {
                return;
            };
            if meta.task_id.as_deref() != Some("task_reconcile_race")
                || meta.subtask_status.as_deref() != Some("running")
            {
                return;
            }
            let opened = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(observer_manager.get_session(meta.id))
            });
            if let Some(sender) = observed_tx.lock().expect("observer sender lock").take() {
                sender
                    .send(opened.map(|session| session.runtime.subtask.status))
                    .expect("record synchronously opened status");
            }
        }));

    let response = manager
        .run_subtask(SessionSubtaskRequest {
            parent_session_id: parent.id,
            description: "race fixture".to_owned(),
            prompt: "finish the fixture".to_owned(),
            access: agena_domain::ExecutionAccess::Inherit,
            skills: None,
            task_id: Some("task_reconcile_race".to_owned()),
            requested_model_selection: agena_domain::ModelSelectionConfig {
                provider: Some("fake".to_owned()),
                model: Some("fake-model".to_owned()),
                ..Default::default()
            },
            timeout_ms: Some(5_000),
            max_tokens: None,
            max_cost_microusd: None,
        })
        .await
        .expect("run subtask through the forced open race");

    let opened_status = observed_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("Running publication observer fired")
        .expect("opening the child succeeds");
    assert_eq!(
        opened_status,
        agena_domain::SubtaskStatus::Running,
        "a child owned by this live launch must not be rewritten as Interrupted"
    );
    assert_eq!(response.status, agena_domain::SubtaskStatus::Completed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subtask_timeout_persists_a_complete_timeout_failure_even_when_cleanup_is_cancelled() {
    let provider = Arc::new(HangingProvider {
        model: ModelId::new("hanging-model"),
    });
    let manager = manager_with_provider(provider).await;
    let parent = create_with_model(
        &manager,
        "subtask timeout parent",
        "hanging",
        "hanging-model",
    )
    .await;

    let response = manager
        .run_subtask(SessionSubtaskRequest {
            parent_session_id: parent.id,
            description: "BG child timeout fixture".to_owned(),
            prompt: "wait forever".to_owned(),
            access: agena_domain::ExecutionAccess::Inherit,
            skills: None,
            task_id: Some("task_timeout_fixture".to_owned()),
            requested_model_selection: agena_domain::ModelSelectionConfig {
                provider: Some("hanging".to_owned()),
                model: Some("hanging-model".to_owned()),
                ..Default::default()
            },
            timeout_ms: Some(20),
            max_tokens: None,
            max_cost_microusd: None,
        })
        .await
        .expect("timeout terminal state must satisfy the database invariant");

    assert_eq!(response.status, agena_domain::SubtaskStatus::TimedOut);
    let failure = response.failure.expect("timeout carries a failure");
    assert_eq!(failure.code.as_str(), "subtask.timeout");
    assert!(failure.user.fallback.contains("task_timeout_fixture"));
    assert!(failure.user.fallback.contains("20 ms"));
    let persisted = manager
        .session_store()
        .load(response.session.id)
        .await
        .expect("reload timed-out child");
    assert_eq!(persisted.meta.subtask_status.as_deref(), Some("timed_out"));
    let persisted_failure = persisted
        .meta
        .subtask_failure
        .expect("durable timeout failure");
    assert_eq!(
        persisted_failure
            .get("code")
            .and_then(serde_json::Value::as_str),
        Some("subtask.timeout")
    );
}

#[derive(Default)]
struct ToolSearchFixture;

#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    agena_plugin_host::sdk::ToolInput,
)]
#[input(non_empty("tool"))]
#[serde(deny_unknown_fields)]
struct ToolHelpFixtureInput {
    tool: String,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "tools",
    version = "test",
    summary = "Tool API stable-loop regression fixture."
)]
impl ToolSearchFixture {
    #[tool(
        name = "search",
        summary = "Search live tools.",
        read_only,
        discovery,
        concurrency_safe
    )]
    async fn search(
        &self,
        input: &agena_runtime_contracts::part::ToolSearchToolInput,
    ) -> agena_plugin_host::sdk::ToolInvokeOutput {
        agena_plugin_host::sdk::ToolInvokeOutput::text(format!(
            "Matching tools for {:?}:\n- fs.read [filesystem, query]: Read workspace files.",
            input.query
        ))
    }

    #[tool(
        name = "help",
        summary = "Inspect one live tool contract.",
        read_only,
        discovery,
        concurrency_safe
    )]
    async fn help(
        &self,
        input: &ToolHelpFixtureInput,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        if input.tool == "fs.list" {
            return Err(agena_plugin_host::sdk::PluginError::invalid_params(
                "unknown tool 'fs.list'",
            ));
        }
        Ok(agena_plugin_host::sdk::ToolInvokeOutput::text(format!(
            "Contract for {}",
            input.tool
        )))
    }
}

#[derive(Default)]
struct FilesystemFixture;

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "fs",
    version = "test",
    summary = "Filesystem catalog regression fixture."
)]
impl FilesystemFixture {
    #[tool(name = "read", summary = "Read workspace files.", read_only)]
    async fn read(&self) -> String {
        "fixture".to_owned()
    }
}

/// Stateful fake provider for the complete model -> tools_search -> model
/// loop. The first request emits reasoning plus one gateway call; the second
/// captures the replayed transcript and emits the final answer.
struct ToolSearchLoopProvider {
    model: ModelId,
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
}

/// Holds the first provider request open so an Assistant-owned background hook
/// can be committed before the current part reaches the stable-loop boundary.
/// The second request is the notification response. This deterministically
/// reproduces the fast-background-completion ordering observed with a real
/// `shell.run`.
struct AssistantHookBoundaryProvider {
    model: ModelId,
    first_request_uses_tool: bool,
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
    first_request_entered: tokio::sync::Notify,
    release_first_request: tokio::sync::Notify,
}

impl ToolSearchLoopProvider {
    fn new() -> Self {
        Self {
            model: ModelId::new("tool-loop-model"),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

impl AssistantHookBoundaryProvider {
    fn new() -> Self {
        Self {
            model: ModelId::new("assistant-hook-boundary-model"),
            first_request_uses_tool: true,
            requests: std::sync::Mutex::new(Vec::new()),
            first_request_entered: tokio::sync::Notify::new(),
            release_first_request: tokio::sync::Notify::new(),
        }
    }

    fn text_only() -> Self {
        Self {
            first_request_uses_tool: false,
            ..Self::new()
        }
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

#[async_trait::async_trait]
impl ModelRuntime for ToolSearchLoopProvider {
    fn id(&self) -> &str {
        "tool-loop"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    fn agena_tool_mode(&self, _model: &ModelId) -> agena_provider::AgenaToolMode {
        agena_provider::AgenaToolMode::ProviderProtocol
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
            text: "TOOL_SEARCH_OK fs.read".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
        })
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        let request_index = {
            let mut requests = self.requests.lock().expect("request lock");
            let request_index = requests.len();
            requests.push(request);
            request_index
        };
        let provider_id = ProviderId::new(self.id());
        let model = self.model.clone();
        let events = match request_index {
            0 => vec![
                Ok(CompletionStreamEvent::ThinkingDelta {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    delta: "I must discover the filesystem tool.".to_owned(),
                }),
                Ok(CompletionStreamEvent::ToolCallSnapshot {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    stream_key: "call:0".to_owned(),
                    id: Some("call_tools_search_1".to_owned()),
                    name: Some("tools_search".to_owned()),
                    arguments_json: r#"{"query":"filesystem"}"#.to_owned(),
                }),
                // Some gateways repeat registration metadata in a trailer but
                // serialize an absent name as whitespace. The valid earlier
                // function identity must remain authoritative.
                Ok(CompletionStreamEvent::ToolCallDelta {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    stream_key: "call:0".to_owned(),
                    id: Some("   ".to_owned()),
                    name: Some("   ".to_owned()),
                    arguments_delta: String::new(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id,
                    model,
                    finish_reason: Some(CompletionFinishReason::ToolCalls),
                    usage: Some(CompletionUsage::default()),
                    // Match OpenAI-compatible gateways that return plaintext
                    // Responses reasoning items but omit the explicit
                    // `assistant_reasoning_field` hint.
                    provider_metadata: Some(serde_json::json!({
                        "openai_reasoning_items": [{
                            "type": "reasoning",
                            "summary": [],
                            "content": [{
                                "type": "reasoning_text",
                                "text": "I must discover the filesystem tool."
                            }]
                        }]
                    })),
                    end_turn: None,
                }),
            ],
            1 => vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    delta: "TOOL_SEARCH_OK fs.read".to_owned(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id,
                    model,
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: Some(CompletionUsage::default()),
                    provider_metadata: None,
                    end_turn: None,
                }),
            ],
            other => panic!("stable run made an unexpected provider request #{other}"),
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

#[async_trait::async_trait]
impl ModelRuntime for AssistantHookBoundaryProvider {
    fn id(&self) -> &str {
        "assistant-hook-boundary"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    fn agena_tool_mode(&self, _model: &ModelId) -> agena_provider::AgenaToolMode {
        agena_provider::AgenaToolMode::ProviderProtocol
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
            text: "BG_BOUNDARY_NOTIFICATION_RECEIVED".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
        })
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        let request_index = {
            let mut requests = self.requests.lock().expect("request lock");
            let request_index = requests.len();
            requests.push(request);
            request_index
        };
        let provider_id = ProviderId::new(self.id());
        let model = self.model.clone();
        let events = match request_index {
            0 => {
                self.first_request_entered.notify_one();
                self.release_first_request.notified().await;
                if self.first_request_uses_tool {
                    vec![
                        Ok(CompletionStreamEvent::ToolCallSnapshot {
                            provider_id: provider_id.clone(),
                            model: model.clone(),
                            stream_key: "call:boundary-tool".to_owned(),
                            id: Some("call_boundary_tool".to_owned()),
                            name: Some("tools_search".to_owned()),
                            arguments_json: r#"{"query":"filesystem"}"#.to_owned(),
                        }),
                        Ok(CompletionStreamEvent::Completed {
                            provider_id,
                            model,
                            finish_reason: Some(CompletionFinishReason::ToolCalls),
                            usage: Some(CompletionUsage::default()),
                            provider_metadata: None,
                            end_turn: None,
                        }),
                    ]
                } else {
                    vec![
                        Ok(CompletionStreamEvent::TextDelta {
                            provider_id: provider_id.clone(),
                            model: model.clone(),
                            delta: "FIRST_PROVIDER_PART_COMPLETED".to_owned(),
                        }),
                        Ok(CompletionStreamEvent::Completed {
                            provider_id,
                            model,
                            finish_reason: Some(CompletionFinishReason::Stop),
                            usage: Some(CompletionUsage::default()),
                            provider_metadata: None,
                            end_turn: None,
                        }),
                    ]
                }
            }
            1 => vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    delta: "BG_BOUNDARY_NOTIFICATION_RECEIVED".to_owned(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id,
                    model,
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: Some(CompletionUsage::default()),
                    provider_metadata: None,
                    end_turn: None,
                }),
            ],
            other => {
                panic!("assistant-hook-boundary run made an unexpected provider request #{other}")
            }
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// Reproduces the production failure that wedged session 25: two
/// concurrency-safe Tool API calls finish in one batch, one Failed and one
/// Completed. The failed terminal part must not be projected as pending on the
/// next stable-loop iteration.
struct MixedToolHelpBatchProvider {
    model: ModelId,
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
}

impl MixedToolHelpBatchProvider {
    fn new() -> Self {
        Self {
            model: ModelId::new("mixed-tool-help-model"),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("request lock").len()
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

#[async_trait::async_trait]
impl ModelRuntime for MixedToolHelpBatchProvider {
    fn id(&self) -> &str {
        "mixed-tool-help"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    fn agena_tool_mode(&self, _model: &ModelId) -> agena_provider::AgenaToolMode {
        agena_provider::AgenaToolMode::ProviderProtocol
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
            text: "MIXED_TOOL_BATCH_OK".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
        })
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        let request_index = {
            let mut requests = self.requests.lock().expect("request lock");
            let request_index = requests.len();
            requests.push(request);
            request_index
        };
        let provider_id = ProviderId::new(self.id());
        let model = self.model.clone();
        let events = match request_index {
            0 => vec![
                Ok(CompletionStreamEvent::ToolCallSnapshot {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    stream_key: "call:invalid-help".to_owned(),
                    id: Some("call_invalid_help".to_owned()),
                    name: Some("tools_help".to_owned()),
                    arguments_json: r#"{"tool":"fs.list"}"#.to_owned(),
                }),
                Ok(CompletionStreamEvent::ToolCallSnapshot {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    stream_key: "call:valid-help".to_owned(),
                    id: Some("call_valid_help".to_owned()),
                    name: Some("tools_help".to_owned()),
                    arguments_json: r#"{"tool":"fs.read"}"#.to_owned(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id,
                    model,
                    finish_reason: Some(CompletionFinishReason::ToolCalls),
                    usage: Some(CompletionUsage::default()),
                    provider_metadata: None,
                    end_turn: None,
                }),
            ],
            1 => vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    delta: "MIXED_TOOL_BATCH_OK".to_owned(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id,
                    model,
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: Some(CompletionUsage::default()),
                    provider_metadata: None,
                    end_turn: None,
                }),
            ],
            other => panic!("stable run made an unexpected provider request #{other}"),
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

async fn manager_with_tool_search_fixture(provider: Arc<dyn ModelRuntime>) -> SessionManager {
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let mut plugins_config = PluginsConfig::default();
    for plugin_id in ["agena.tools", "agena.fs"] {
        plugins_config
            .list
            .insert(plugin_id.to_owned(), ConfiguredPlugin::static_default());
    }
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new(
                "agena.tools".parse().expect("valid Tool API plugin key"),
                ToolSearchFixture,
            ),
            StaticPluginRegistration::new(
                "agena.fs".parse().expect("valid filesystem plugin key"),
                FilesystemFixture,
            ),
        ],
        config: plugins_config,
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build Tool API plugin host");
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        Arc::clone(&plugins),
        None,
        None,
        None,
    );
    let mut registry = ProviderRegistry::new();
    registry.register_arc(provider);
    let provider_registry = Arc::new(registry);
    let context_governor = ContextGovernor::new(agena_domain::ContextPolicy::default());
    let processor = SessionProcessor::new(plugins);
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("open v2 test database");
    initialize(&database).await;
    SessionManager::new(
        database,
        provider_registry,
        context_governor,
        processor,
        executor,
        RuntimeSessionManagerConfig::default(),
    )
}

#[tokio::test]
async fn processor_run_turn_streams_parts_through_the_facade_without_v1_double_write() {
    // 25 deltas > STREAMING_FLUSH_DELTA_COUNT (8): the facade must amortize
    // them into coalesced flushes rather than one revision per token.
    let tokens: Vec<String> = (0..25).map(|i| format!("tok{i} ")).collect();
    let full_text: String = tokens.concat();
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: tokens,
        thinking_deltas: vec!["think one".to_owned()],
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session = create(&manager, "streamed turn").await;

    // The caller starts the run marker before the provider turn (17.4).
    let turn_id = TurnId::new();
    let reply_id = AssistantReplyId::new();
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content(
                "continue",
                Some("fake"),
                Some("fake-model"),
                Some(turn_id),
                Some(reply_id),
            ),
        )
        .await
        .expect("start run marker");

    let run = SessionRunRequest {
        session_id: session.id,
        model: ModelRef::new("fake", "fake-model"),
        completion: CompletionRequest {
            model: ModelId::new("fake-model"),
            system: None,
            turns: Vec::new(),
            tool_api_functions: Vec::new(),
            provider_native_tools: Default::default(),
            disable_tools: false,
            temperature: None,
            max_output_tokens: None,
            prompt_cache_key: None,
            previous_response_id: None,
            prompt_window_generation: None,
            provider_compaction: None,
            stop_sequences: Vec::new(),
            top_p: None,
            top_k: None,
            seed: None,
            thinking: None,
            verbosity: None,
            response_format: None,
            responses_api_metadata: None,
            request_override: Default::default(),
        },
        next_message_id: run_id,
        marker_content: run_marker_content("continue", None, None, None, None),
        input_notification_part_ids: Vec::new(),
        part_ids: ProcessorPartIdAllocator,
        next_call_id: 0,
        store: manager.store.clone(),
        cancel: None,
    };

    let state = manager.execution_state();
    let result = state
        .processor
        .run_turn(run, &state.provider_registry)
        .await
        .expect("stream one assistant turn through the parts-native processor");

    // Terminal Completed: the marker and every content part end terminal.
    assert!(matches!(
        result.termination,
        SessionRunTermination::Completed
    ));
    assert_eq!(result.run_marker.part_id, run_id);
    assert_eq!(result.run_marker.kind, "run");
    assert_eq!(result.run_marker.state, PartState::Completed);

    // Exactly one text part and one think part under the marker.
    let text_parts: Vec<_> = result
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .collect();
    assert_eq!(text_parts.len(), 1, "streamed text coalesces into one part");
    assert_eq!(text_parts[0].state, PartState::Completed);
    assert_eq!(text_parts[0].run_id, Some(run_id));
    assert_eq!(text_parts[0].content["text"], full_text);
    assert!(
        text_parts[0].revision < 25,
        "D10 amortizes deltas into coalesced flushes: 25 deltas produced revision {}",
        text_parts[0].revision
    );
    let think_parts: Vec<_> = result
        .parts
        .iter()
        .filter(|part| part.kind == "think")
        .collect();
    assert_eq!(
        think_parts.len(),
        1,
        "thinking delta becomes one think part"
    );
    assert_eq!(think_parts[0].state, PartState::Completed);

    // No v1 double-write: the facade holds exactly the marker plus the parts
    // this turn created — no duplicate rows and no v1 message artifacts.
    let view = manager
        .session_store()
        .load(session.id)
        .await
        .expect("load facade view");
    assert_eq!(view.parts.len(), 3, "run marker + think + text only");
    assert!(
        view.parts
            .iter()
            .all(|part| part.kind == "run" || part.run_id == Some(run_id)),
        "every content part hangs off the run marker"
    );

    // A mid-stream InProgress part exposes its buffered partial text through
    // load() before any flush (the facade's streaming-buffer overlay) — the
    // exact per-token `update_part(content_text_delta)` path the processor
    // drives, amortized in the buffer rather than committed per token.
    let run2 = manager
        .store
        .start_run(
            session.id,
            "continue",
            serde_json::json!({"run_kind": "continue"}),
        )
        .await
        .expect("start second marker");
    let created = manager
        .store
        .append_parts(
            session.id,
            run2,
            vec![NewPart {
                kind: "text".to_owned(),
                role: PartRole::Assistant,
                content: serde_json::json!({ "type": "text", "text": "" }),
                summary: None,
                visibility: PartVisibility::Both,
                parent_part_id: None,
                state: PartState::InProgress,
            }],
        )
        .await
        .expect("append in-progress text part");
    let part_id = created[0].part_id;
    for chunk in ["live", " ", "delta", "s"] {
        manager
            .store
            .update_part(
                session.id,
                part_id,
                PartDelta {
                    content_text_delta: Some(chunk.to_owned()),
                    ..Default::default()
                },
            )
            .await
            .expect("push stream delta");
    }
    let mid = manager
        .session_store()
        .load(session.id)
        .await
        .expect("load mid-stream view");
    let mid_part = mid
        .parts
        .iter()
        .find(|part| part.part_id == part_id)
        .expect("mid-stream part");
    assert_eq!(mid_part.state, PartState::InProgress);
    assert_eq!(mid_part.content["text"], "live deltas");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stable_run_executes_in_progress_tools_search_and_replays_reasoning() {
    let provider = Arc::new(ToolSearchLoopProvider::new());
    let manager = manager_with_tool_search_fixture(provider.clone()).await;
    let session = create(&manager, "tools_search stable loop").await;
    let request = SessionUserRunRequest::new(
        session.id,
        agena_runtime::SessionRunOptions {
            model: ModelRef::new("tool-loop", "tool-loop-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: Some("Call the requested Tool API function.".to_owned()),
            temperature: Some(0.0),
            max_output_tokens: Some(256),
        },
        vec![TypedContent::Text(text_content(
            "Call tools_search for filesystem tools.",
        ))],
    );

    let completed = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        manager.submit_subtask_user_message(request, None),
    )
    .await
    .expect("model -> tool -> model loop must not hang")
    .expect("stable run must complete");

    let tool_part = completed
        .parts()
        .iter()
        .find(|part| part.kind == "tool_call")
        .unwrap_or_else(|| panic!("tools_search part missing from {:#?}", completed.parts()));
    assert_eq!(
        tool_part.state,
        PartState::Completed,
        "the processor creates streamed calls as InProgress; the singleton sequential path must execute them"
    );
    let TypedContent::ToolCall(tool_call) =
        crate::session::store::typed_content_from_value(&tool_part.kind, &tool_part.content)
            .expect("decode tools_search part")
    else {
        panic!("expected tool_call content");
    };
    assert!(
        tool_call.output.is_some(),
        "completed tool call keeps its raw output"
    );
    assert!(completed.parts().iter().any(|part| {
        part.kind == "text"
            && part.content.get("text").and_then(serde_json::Value::as_str)
                == Some("TOOL_SEARCH_OK fs.read")
    }));

    let requests = provider.requests();
    assert_eq!(
        requests.len(),
        2,
        "the persisted pending call must keep the stable loop alive for one follow-up turn"
    );
    assert!(
        requests[0]
            .tool_api_functions
            .iter()
            .any(|tool| tool.name == "tools_search"),
        "tools_search must be advertised to the model"
    );
    let replayed_tool_turn = requests[1]
        .turns
        .iter()
        .find(|run| {
            run.role == Role::Assistant
                && run.parts.iter().any(|part| {
                    matches!(
                        part,
                        CompletionInputPart::ToolCall { function, .. }
                            if function.function_name() == "tools_search"
                    )
                })
        })
        .expect("second request must replay the assistant tool-calling turn");
    assert_eq!(
        replayed_tool_turn.provider_state.assistant_reasoning_field,
        Some(AssistantReasoningField::ReasoningContent),
        "provider replay state must be persisted before the tool resolves"
    );
    assert_eq!(
        replayed_tool_turn
            .provider_state
            .openai_reasoning_items
            .len(),
        1
    );
    assert!(replayed_tool_turn.parts.iter().any(|part| {
        matches!(part, CompletionInputPart::Reasoning { text } if text == "I must discover the filesystem tool.")
    }));
    let replayed_call_id = replayed_tool_turn
        .parts
        .iter()
        .find_map(|part| match part {
            CompletionInputPart::ToolCall { id, function, .. }
                if function.function_name() == "tools_search" =>
            {
                Some(id.as_str())
            }
            _ => None,
        })
        .expect("replayed tools_search call id");
    assert_eq!(
        replayed_call_id, "call_tools_search_1",
        "the persisted replay must keep the provider-issued call id rather than replacing it with the local call sequence"
    );
    let replayed_results: Vec<&String> = replayed_tool_turn
        .parts
        .iter()
        .filter_map(|part| match part {
            CompletionInputPart::ToolResult {
                tool_call_id,
                output_json,
                ..
            } if tool_call_id == replayed_call_id => Some(output_json),
            _ => None,
        })
        .collect();
    eprintln!("DEBUG replay output_json: {:?}", replayed_results);
    assert!(
        replayed_tool_turn.parts.iter().any(|part| {
            matches!(
                part,
                CompletionInputPart::ToolResult {
                    tool_call_id,
                    output_json,
                    ..
                } if tool_call_id == replayed_call_id
                    && output_json.contains("fs.read")
            )
        }),
        "replayed parts: {:#?}",
        replayed_tool_turn.parts
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stable_run_continues_after_mixed_failed_and_completed_parallel_tool_batch() {
    let provider = Arc::new(MixedToolHelpBatchProvider::new());
    let manager = manager_with_tool_search_fixture(provider.clone()).await;
    let session = create(&manager, "mixed Tool API batch").await;
    let mut request_override = agena_domain::ModelSpeedModeRequestOverride::default();
    request_override.set_parallel_tool_calls(Some(true));
    let request = SessionUserRunRequest::new(
        session.id,
        agena_runtime::SessionRunOptions {
            model: ModelRef::new("mixed-tool-help", "mixed-tool-help-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override,
            system: Some("Inspect both tool contracts, then finish.".to_owned()),
            temperature: Some(0.0),
            max_output_tokens: Some(256),
        },
        vec![TypedContent::Text(text_content(
            "Inspect one invalid and one valid tool contract.",
        ))],
    );

    let completed = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        manager.submit_subtask_user_message(request, None),
    )
    .await
    .expect("mixed terminal tool batch must not spin forever")
    .expect("stable run must continue after returning the failed tool result to the model");

    let tool_states = completed
        .parts()
        .iter()
        .filter(|part| part.kind == "tool_call")
        .map(|part| part.state)
        .collect::<Vec<_>>();
    assert_eq!(tool_states.len(), 2);
    assert!(tool_states.contains(&PartState::Failed));
    assert!(tool_states.contains(&PartState::Completed));
    assert!(
        completed.pending_tools().is_empty(),
        "failed and completed tool calls are both terminal"
    );
    assert_eq!(
        provider.request_count(),
        2,
        "the terminal batch must trigger exactly one follow-up model turn"
    );
    let requests = provider.requests();
    let replayed_tool_turn = requests[1]
        .turns
        .iter()
        .find(|run| {
            run.role == Role::Assistant
                && run.parts.iter().any(|part| {
                    matches!(part, CompletionInputPart::ToolCall { function, .. }
                        if function.function_name() == "tools_help")
                })
        })
        .expect("second request must replay the mixed tool-calling turn");
    for expected_id in ["call_invalid_help", "call_valid_help"] {
        assert!(
            replayed_tool_turn.parts.iter().any(|part| {
                matches!(part, CompletionInputPart::ToolCall { id, .. } if id == expected_id)
            }),
            "the replayed function call must keep provider id {expected_id}: {:#?}",
            replayed_tool_turn.parts
        );
        assert!(
            replayed_tool_turn.parts.iter().any(|part| {
                matches!(part, CompletionInputPart::ToolResult { tool_call_id, .. }
                    if tool_call_id == expected_id)
            }),
            "the replayed terminal result must correlate with provider id {expected_id}: {:#?}",
            replayed_tool_turn.parts
        );
    }
    assert!(completed.parts().iter().any(|part| {
        part.kind == "text"
            && part.content.get("text").and_then(serde_json::Value::as_str)
                == Some("MIXED_TOOL_BATCH_OK")
    }));
}

#[tokio::test]
async fn policy_denied_terminal_transition_preserves_operation_metadata() {
    let manager = test_manager().await;
    let session = create(&manager, "policy-denied operation identity").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .expect("start assistant run");
    let mut operation = agena_runtime_contracts::part::OperationPart::pending(
        7,
        ToolInvocation::new("fixture.denied", StructuredObject::default()),
        TimeRange {
            start_ms: 1,
            end_ms: None,
        },
    );
    operation.metadata.insert(
        OPERATION_ID_METADATA_KEY.to_owned(),
        serde_json::json!("call_policy_denied_1"),
    );
    operation.metadata.insert(
        "fixture.runtime_context".to_owned(),
        serde_json::json!({"preserve": true}),
    );
    let tool_part = new_part_from_content(
        "tool_call",
        PartRole::Assistant,
        &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation))),
        PartState::InProgress,
    )
    .expect("build pending tool part");
    let created = manager
        .store
        .append_parts(session.id, run_id, vec![tool_part])
        .await
        .expect("append pending tool part");
    let tool_part_id = created[0].part_id;
    let session = manager
        .store
        .load_session(session.id)
        .await
        .expect("load pending tool");
    let pending = session
        .pending_tool_by_part_id(tool_part_id)
        .expect("resolve pending tool");
    let session_id = session.id;

    manager
        .apply_tool_policy_denied(
            session,
            &pending,
            agena_domain::PolicyDeniedResult {
                action: agena_domain::PermissionAction::Tool {
                    tool_name: "fixture.denied".to_owned(),
                    qualifier: None,
                },
                related_actions: Vec::new(),
                denied_actions: Vec::new(),
                reason: "fixture policy denial".to_owned(),
                explanation: String::new(),
                source: Some("test".to_owned()),
                scope: None,
                operator: None,
                authority: agena_domain::PermissionAuthorityKind::StaticPolicy,
                rule_id: None,
                rule_revision_ms: None,
                trace: Vec::new(),
            },
            manager.execution_state(),
        )
        .await
        .expect("persist policy-denied terminal result");

    let reloaded = manager
        .store
        .load_session(session_id)
        .await
        .expect("reload terminal tool");
    let tool_part = reloaded
        .parts()
        .iter()
        .find(|part| part.part_id == tool_part_id)
        .expect("terminal tool part");
    assert!(tool_part.state.is_terminal());
    let operation = operation_from_part(tool_part).expect("decode terminal operation");
    assert_eq!(
        operation
            .metadata
            .get(OPERATION_ID_METADATA_KEY)
            .and_then(serde_json::Value::as_str),
        Some("call_policy_denied_1")
    );
    assert_eq!(
        operation.metadata.get("fixture.runtime_context"),
        Some(&serde_json::json!({"preserve": true})),
        "terminal constructors must inherit all runtime metadata, not only the provider call id"
    );
}

#[tokio::test]
async fn idless_plugin_asks_are_unique_and_cancellation_is_scoped_and_model_visible() {
    let manager = test_manager().await;
    let session = create(&manager, "idless plugin ask correlation").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .expect("start assistant run");
    let operation = |call_id, name: &str| {
        agena_runtime_contracts::part::OperationPart::pending(
            call_id,
            ToolInvocation::new(name, StructuredObject::default()),
            TimeRange {
                start_ms: 1,
                end_ms: None,
            },
        )
    };
    let created = manager
        .store
        .append_parts(
            session.id,
            run_id,
            vec![
                new_part_from_content(
                    "tool_call",
                    PartRole::Assistant,
                    &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation(
                        77,
                        "fixture.ask.first",
                    )))),
                    PartState::InProgress,
                )
                .expect("build first pending tool"),
                new_part_from_content(
                    "tool_call",
                    PartRole::Assistant,
                    &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation(
                        78,
                        "fixture.ask.second",
                    )))),
                    PartState::InProgress,
                )
                .expect("build second pending tool"),
            ],
        )
        .await
        .expect("append idless pending tools");
    let first_part_id = created[0].part_id;
    let second_part_id = created[1].part_id;
    let mut session = manager
        .store
        .load_session(session.id)
        .await
        .expect("load pending tools");

    for (part_id, title) in [
        (first_part_id, "First question"),
        (second_part_id, "Second question"),
    ] {
        let pending = session
            .pending_tool_by_part_id(part_id)
            .expect("resolve idless pending tool");
        session = manager
            .apply_tool_execution_result(
                session,
                &pending,
                Err(crate::tool::ToolError::UserInputRequired(Box::new(
                    crate::part::AskUserToolInput {
                        title: title.to_owned(),
                        kind: "ask_user".to_owned(),
                        body_markdown: String::new(),
                        auto_resolution_ms: None,
                        questions: Vec::new(),
                    },
                ))),
                manager.execution_state(),
            )
            .await
            .expect("persist plugin user-input request");
    }

    let first_request_id = session
        .parts()
        .iter()
        .find(|part| part.part_id == first_part_id)
        .and_then(tool_part_first_user_input)
        .map(|record| record.request.request_id)
        .expect("first plugin request id");
    let second_request_id = session
        .parts()
        .iter()
        .find(|part| part.part_id == second_part_id)
        .and_then(tool_part_first_user_input)
        .map(|record| record.request.request_id)
        .expect("second plugin request id");
    assert_eq!(first_request_id, format!("tool-input:{}:77", session.id));
    assert_eq!(second_request_id, format!("tool-input:{}:78", session.id));
    assert_ne!(
        first_request_id, second_request_id,
        "id-less provider calls must not collapse unrelated plugin asks into an empty request id"
    );

    let first_pending = session
        .pending_tool_by_part_id(first_part_id)
        .expect("first tool remains pending while awaiting input");
    session = manager
        .apply_tool_cancellation(session, &first_pending, manager.execution_state())
        .await
        .expect("cancel first idless tool");
    let first_part = session
        .parts()
        .iter()
        .find(|part| part.part_id == first_part_id)
        .expect("cancelled first tool part");
    let first_operation = operation_from_part(first_part).expect("decode cancelled operation");
    assert_eq!(first_part.state, PartState::Cancelled);
    assert_eq!(
        first_operation.status(),
        agena_domain::ExecutionStatus::Cancelled
    );
    assert!(
        first_operation.user_input.requests.is_empty(),
        "the cancelled tool must not retain an unanswerable pending request"
    );
    assert!(
        super::replies::has_finished_operation(&session, first_request_id.as_str()),
        "a late retry of the removed id-less ask must still resolve as a duplicate of the terminal operation"
    );
    assert!(
        crate::provider::project_operation_output(first_operation.status(), &first_operation)
            .contains("cancelled"),
        "a cancelled function call must replay a non-empty result to the model"
    );

    let second_part = session
        .parts()
        .iter()
        .find(|part| part.part_id == second_part_id)
        .expect("second tool part");
    assert!(second_part.state.is_in_flight());
    assert_eq!(
        tool_part_first_user_input(second_part)
            .map(|record| record.request.request_id)
            .as_deref(),
        Some(second_request_id.as_str()),
        "terminalizing one id-less operation must not clear another operation's ask"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provider_native_completion_preserves_started_identity_and_context() {
    let provider = Arc::new(ProviderNativeFixtureProvider {
        model: ModelId::new("provider-native-model"),
        mode: ProviderNativeFixtureMode::CompletionOmitsId,
    });
    let manager = manager_with_provider(provider).await;
    let session = create(&manager, "provider-native identity").await;
    let completed = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        manager.submit_subtask_user_message(
            SessionUserRunRequest::new(
                session.id,
                agena_runtime::SessionRunOptions {
                    model: ModelRef::new("provider-native-fixture", "provider-native-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: Some(0.0),
                    max_output_tokens: Some(64),
                },
                vec![TypedContent::Text(text_content("run hosted search"))],
            ),
            None,
        ),
    )
    .await
    .expect("provider-native run must terminate")
    .expect("provider-native run succeeds");

    let tool_parts = completed
        .parts()
        .iter()
        .filter(|part| part.kind == "tool_call")
        .collect::<Vec<_>>();
    assert_eq!(tool_parts.len(), 1, "one hosted call produces one activity");
    let part = tool_parts[0];
    assert_eq!(part.state, PartState::Completed);
    let operation = operation_from_part(part).expect("decode provider-native operation");
    assert!(operation.is_provider_only());
    assert_eq!(operation.status(), agena_domain::ExecutionStatus::Completed);
    assert_eq!(
        operation
            .metadata
            .get(OPERATION_ID_METADATA_KEY)
            .and_then(serde_json::Value::as_str),
        Some("ws_fixture_1"),
        "a completion event that omits id must retain the started event's provider identity"
    );
    assert_eq!(operation.title(), Some(operation.invocation.name.as_str()));
    assert_eq!(
        operation
            .invocation
            .input
            .get("query")
            .and_then(agena_domain::StructuredValue::as_text),
        Some("fixture")
    );
    assert_eq!(
        operation.provider_raw().and_then(|raw| raw["id"].as_str()),
        Some("ws_fixture_1")
    );
    assert_eq!(
        operation
            .provider_raw()
            .and_then(|raw| raw["result"].as_str()),
        Some("done")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unfinished_provider_native_call_fails_instead_of_wedging_the_run() {
    let provider = Arc::new(ProviderNativeFixtureProvider {
        model: ModelId::new("provider-native-model"),
        mode: ProviderNativeFixtureMode::StartedOnly,
    });
    let manager = manager_with_provider(provider).await;
    let session = create(&manager, "unfinished provider-native call").await;
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        manager.submit_subtask_user_message(
            SessionUserRunRequest::new(
                session.id,
                agena_runtime::SessionRunOptions {
                    model: ModelRef::new("provider-native-fixture", "provider-native-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: Some(0.0),
                    max_output_tokens: Some(64),
                },
                vec![TypedContent::Text(text_content(
                    "start incomplete hosted search",
                ))],
            ),
            None,
        ),
    )
    .await
    .expect("malformed provider-native run must terminate")
    .expect_err("a started-only hosted call is a provider protocol error");
    assert!(
        error
            .to_string()
            .contains("unfinished provider-native tool calls")
    );
    let reloaded = manager
        .store
        .load_session(session.id)
        .await
        .expect("reload failed provider-native run");
    assert!(
        reloaded
            .parts()
            .iter()
            .all(|part| part.kind != "tool_call" || !part.state.is_in_flight()),
        "a malformed hosted call must not leave an executable ghost operation"
    );
}

#[tokio::test]
async fn provider_startup_failure_persists_expandable_error_part() {
    let provider = Arc::new(StartupFailureProvider {
        model: ModelId::new("failure-model"),
    });
    let manager = manager_with_provider(provider).await;
    let session = create(&manager, "provider failure detail").await;
    let request = SessionUserRunRequest::new(
        session.id,
        agena_runtime::SessionRunOptions {
            model: ModelRef::new("startup-failure", "failure-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: Some(0.0),
            max_output_tokens: Some(64),
        },
        vec![TypedContent::Text(text_content("fail once"))],
    );

    let error = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        manager.submit_subtask_user_message(request, None),
    )
    .await
    .expect("startup failure must terminate instead of hanging")
    .expect_err("fixture provider must fail");
    assert!(
        error
            .to_string()
            .contains("fixture upstream rejected request")
    );

    let reloaded = manager
        .store
        .load_session(session.id)
        .await
        .expect("reload failed run");
    let failed_run = reloaded
        .parts()
        .iter()
        .rev()
        .find(|part| {
            part.kind == "run"
                && part.role == PartRole::Assistant
                && part.state == PartState::Failed
        })
        .expect("failed assistant marker");
    let error_part = reloaded
        .parts()
        .iter()
        .find(|part| part.kind == "error" && part.run_id == Some(failed_run.part_id))
        .expect("failed run must carry a durable error detail part");
    assert_eq!(error_part.state, PartState::Failed);
    let problem: agena_failure::UserProblem = serde_json::from_value(
        error_part
            .content
            .get("problem")
            .cloned()
            .expect("safe user problem payload"),
    )
    .expect("decode safe user problem");
    assert!(
        problem
            .user
            .fallback
            .contains("fixture upstream rejected request"),
        "persisted detail should carry the scrubbed provider cause: {problem:?}"
    );
}

/// A host `ask_user` (e.g. workflow plan approval) that suspends an already
/// executing tool must not downgrade the tool part from `InProgress` back to
/// `Pending`: the part lifecycle is forward-only (17.2) and the store rejects
/// `in_progress -> pending` (regression: plan.set blew up with "invalid part
/// transition in_progress -> pending" and dumped the review dialog payload).
#[tokio::test]
async fn host_user_input_does_not_downgrade_an_in_progress_tool_part() {
    let (manager, _database) = test_manager_with_database().await;
    let session = create(&manager, "host ask_user on in-progress tool").await;

    // A tool_call part that has already started executing (state = InProgress),
    // as produced by `resolve_pending_tool` just before streaming execution.
    let operation = agena_runtime_contracts::part::OperationPart::pending(
        1,
        ToolInvocation::new("session.rename", StructuredObject::default()),
        TimeRange {
            start_ms: 1,
            end_ms: None,
        },
    );
    let tool_part = new_part_from_content(
        "tool_call",
        PartRole::Assistant,
        &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation))),
        PartState::InProgress,
    )
    .expect("build tool part");
    let store = manager.session_store();
    let outcome = store
        .submit_user_run(
            session.id,
            manager.store.owner_id.as_str(),
            vec![tool_part],
            None,
        )
        .await
        .expect("submit run with in-progress tool part");
    let tool_part_id = outcome.parts[1].part_id;
    // Reload so the session projection carries the submitted run and the tool
    // part can be resolved by id, exactly as `request_host_user_input` does.
    let session = manager
        .store
        .load_session(session.id)
        .await
        .expect("reload submitted run");
    let pending_tool = session
        .pending_tool_by_part_id(tool_part_id)
        .expect("in-progress tool part resolves as a pending tool");

    // The plugin asks the user a question (plan approval) while the tool is
    // mid-execution. This is the exact mutation `request_host_user_input`
    // applies before suspending on the reply; the tool_call part gains a
    // nested user_input record and must survive the suspension without an invalid
    // `in_progress -> pending` downgrade.
    let request = crate::part::AskUserToolInput {
        title: "Approve New Plan".to_owned(),
        kind: "review".to_owned(),
        body_markdown: String::new(),
        auto_resolution_ms: None,
        questions: Vec::new(),
    };
    manager
        .apply_user_input_request_with_id(
            session.clone(),
            &pending_tool,
            request,
            "host-input:1:1:0".to_owned(),
            UserInputSource::Host,
            manager.execution_state(),
        )
        .await
        .expect("requesting host user input must not reject the in-progress tool part");

    let persisted = store.load(session.id).await.expect("reload parts");
    let tool = persisted
        .parts
        .iter()
        .find(|part| part.part_id == tool_part_id)
        .expect("tool part still present");
    assert_eq!(
        tool.state,
        PartState::InProgress,
        "an executing tool suspended on host input must stay InProgress, not be downgraded to Pending"
    );
    assert!(
        tool_part_first_user_input(tool)
            .map(|record| record.request.request_id == "host-input:1:1:0")
            .unwrap_or(false),
        "the host ask_user request is recorded inside the tool operation's user_input bucket"
    );
}

/// A host ask_user nested in a `tool_call` (plan approval) must be resolvable
/// by the reply machinery through its durable request id, so replying to it
/// does not error with "pending user input request not found".
#[tokio::test]
async fn host_ask_user_is_reply_resolvable_on_tool_call() {
    let (manager, _database) = test_manager_with_database().await;
    let session = create(&manager, "host ask_user reply resolution").await;

    let operation = agena_runtime_contracts::part::OperationPart::pending(
        1,
        ToolInvocation::new("plan.set", StructuredObject::default()),
        TimeRange {
            start_ms: 1,
            end_ms: None,
        },
    );
    let tool_part = new_part_from_content(
        "tool_call",
        PartRole::Assistant,
        &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation))),
        PartState::InProgress,
    )
    .expect("build tool part");
    let store = manager.session_store();
    let outcome = store
        .submit_user_run(
            session.id,
            manager.store.owner_id.as_str(),
            vec![tool_part],
            None,
        )
        .await
        .expect("submit run with in-progress tool part");
    let tool_part_id = outcome.parts[1].part_id;
    let session = manager
        .store
        .load_session(session.id)
        .await
        .expect("reload submitted run");
    let pending_tool = session
        .pending_tool_by_part_id(tool_part_id)
        .expect("in-progress tool part resolves as a pending tool");

    let request = crate::part::AskUserToolInput {
        title: "Approve New Plan".to_owned(),
        kind: "review".to_owned(),
        body_markdown: String::new(),
        auto_resolution_ms: None,
        questions: Vec::new(),
    };
    manager
        .apply_user_input_request_with_id(
            session.clone(),
            &pending_tool,
            request,
            "host-input:1:1:0".to_owned(),
            UserInputSource::Host,
            manager.execution_state(),
        )
        .await
        .expect("request host user input");

    let persisted = store.load(session.id).await.expect("reload parts");
    let tool = persisted
        .parts
        .iter()
        .find(|part| part.kind == "tool_call" && part.part_id == tool_part_id)
        .expect("tool part exists");
    assert!(
        persisted
            .parts
            .iter()
            .all(|part| part.kind != "interaction"),
        "one host ask produces exactly one activity: the tool_call operation, no interaction part"
    );
    // The ask lives INSIDE the tool part's operation bucket; the reply
    // machinery correlates on the operation's user_input records.
    let record = tool_part_first_user_input(tool).expect("operation user_input record exists");
    assert_eq!(record.request.request_id, "host-input:1:1:0");

    let session = manager
        .store
        .load_session(session.id)
        .await
        .expect("reload submitted run as a session");
    let pending =
        super::replies::find_pending_user_input_by_request_id(&session, "host-input:1:1:0")
            .expect("reply lookup resolves the awaiting operation record");
    assert_eq!(pending.tool.part.part_id, tool_part_id);
    assert_eq!(
        pending.request.part_id, tool_part_id,
        "the request ref IS the tool_call operation activity in the single-activity shape"
    );
    let resolved_request =
        super::replies::pending_user_input_request(&session, &pending, "host-input:1:1:0")
            .expect("request payload is recoverable");
    assert_eq!(resolved_request.title, "Approve New Plan");
    assert_eq!(resolved_request.kind, agena_domain::UserInputKind::Review);
    assert_eq!(
        resolved_request.source,
        UserInputSource::Host,
        "a host ask_user request carries the typed Host source"
    );
}

/// A non-host user-input reply (Submit/Cancel/Timeout — the request id is the
/// operation id, not a `host-input:` prefix) completes the nested user_input
/// record in-memory and must persist that completion. Regression: the
/// non-host branch only persisted the tool part before the canonical record
/// was updated, so an answered request resurrected as pending on reload.
#[tokio::test]
async fn non_host_user_input_reply_persists_on_tool_call() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["continuing after the answer".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session = create_with_model(
        &manager,
        "non-host ask_user reply durability",
        "fake",
        "fake-model",
    )
    .await;

    // An assistant run marker carrying the canonical turn/reply identity, so
    // the reply continuation can resolve the conversation after the answer is
    // committed (v2 stores the identity in the marker content).
    let turn_id = TurnId::new();
    let reply_id = AssistantReplyId::new();
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content(
                "continue",
                Some("fake"),
                Some("fake-model"),
                Some(turn_id),
                Some(reply_id),
            ),
        )
        .await
        .expect("start assistant run marker");

    let mut operation = agena_runtime_contracts::part::OperationPart::pending(
        1,
        ToolInvocation::new("plan.set", StructuredObject::default()),
        TimeRange {
            start_ms: 1,
            end_ms: None,
        },
    );
    // A plugin ask_user carries the operation id as its request id (exactly
    // what `apply_tool_execution_result` produces), so the request source is
    // `Plugin`.
    operation.metadata.insert(
        OPERATION_ID_METADATA_KEY.to_owned(),
        serde_json::json!("ask-1"),
    );
    let tool_part = new_part_from_content(
        "tool_call",
        PartRole::Assistant,
        &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation))),
        PartState::InProgress,
    )
    .expect("build tool part");
    let created = manager
        .store
        .append_parts(session.id, run_id, vec![tool_part])
        .await
        .expect("append tool part under the assistant marker");
    let tool_part_id = created[0].part_id;
    let session = manager
        .store
        .load_session(session.id)
        .await
        .expect("reload submitted run");
    let pending_tool = session
        .pending_tool_by_part_id(tool_part_id)
        .expect("in-progress tool part resolves as a pending tool");

    // Non-host request id: the operation id, not a "host-input:" prefix.
    let request = crate::part::AskUserToolInput {
        title: "Continue?".to_owned(),
        kind: "ask_user".to_owned(),
        body_markdown: String::new(),
        auto_resolution_ms: None,
        questions: Vec::new(),
    };
    manager
        .apply_user_input_request_with_id(
            session.clone(),
            &pending_tool,
            request,
            "ask-1".to_owned(),
            UserInputSource::Plugin,
            manager.execution_state(),
        )
        .await
        .expect("request user input");

    let persisted = manager
        .session_store()
        .load(session.id)
        .await
        .expect("reload parts after requesting");
    let tool = persisted
        .parts
        .iter()
        .find(|part| part.kind == "tool_call" && part.part_id == tool_part_id)
        .expect("tool part exists");
    assert!(
        persisted
            .parts
            .iter()
            .all(|part| part.kind != "interaction"),
        "one ask produces exactly one activity: the tool_call operation, no interaction part"
    );
    assert!(
        tool.state.is_in_flight(),
        "the request is pending (in flight) before the reply"
    );
    let record = tool_part_first_user_input(tool).expect("operation user_input record exists");
    assert_eq!(record.request.request_id, "ask-1");
    assert_eq!(
        record.request.source,
        UserInputSource::Plugin,
        "a non-host (tool) ask_user carries the typed Plugin source"
    );

    let replied = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        manager.reply_user_input(SessionExecutionReplyRequest::new(
            session.id,
            agena_runtime::SessionRunOptions {
                model: ModelRef::new("fake", "fake-model"),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
            },
            agena_domain::UserInputReply {
                request_id: "ask-1".to_owned(),
                kind: agena_domain::UserInputReplyKind::Submit,
                answers: Default::default(),
                reason: None,
            },
        )),
    )
    .await
    .expect("non-host reply must not hang")
    .expect("non-host reply completes");

    // Reload from the durable store: the tool part must be Completed and its
    // operation record carry the reply payload, not resurrect as pending.
    let persisted = manager
        .session_store()
        .load(session.id)
        .await
        .expect("reload parts after reply");
    let tool = persisted
        .parts
        .iter()
        .find(|part| part.part_id == tool_part_id)
        .expect("tool part remains");
    assert_eq!(
        tool.state,
        PartState::Completed,
        "non-host reply must durably persist the tool operation as Completed"
    );
    let record = tool_part_first_user_input(tool).expect("operation user_input record remains");
    let reply = record
        .reply
        .expect("the replied payload is present on the persisted record");
    assert_eq!(reply.request_id, "ask-1");
    assert_eq!(reply.kind, agena_domain::UserInputReplyKind::Submit);
    assert!(
        !replied.parts().is_empty(),
        "the reply returns the continued session"
    );
}

/// Regression for the born-InProgress lifecycle: the `tool_call` carrying a
/// host ask_user must remain `InProgress` so the reply can complete through
/// the legal `in_progress -> completed` edge. `mark_interactive_request_presented`
/// must also stamp `presented_at` on the nested in-flight request.
#[tokio::test]
async fn host_ask_user_tool_call_born_in_progress_and_reply_completes() {
    let (manager, _database) = test_manager_with_database().await;
    let session = create(&manager, "host ask_user lifecycle").await;

    let operation = agena_runtime_contracts::part::OperationPart::pending(
        1,
        ToolInvocation::new("plan.set", StructuredObject::default()),
        TimeRange {
            start_ms: 1,
            end_ms: None,
        },
    );
    let tool_part = new_part_from_content(
        "tool_call",
        PartRole::Assistant,
        &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation))),
        PartState::InProgress,
    )
    .expect("build tool part");
    let store = manager.session_store();
    store
        .submit_user_run(
            session.id,
            manager.store.owner_id.as_str(),
            vec![tool_part],
            None,
        )
        .await
        .expect("submit run with in-progress tool part");

    // Drive the host ask_user through the public entry point in a background
    // task: it registers the host waiter and blocks awaiting the reply, exactly
    // as the plugin host flow does in production.
    let request = crate::part::AskUserToolInput {
        title: "Approve New Plan".to_owned(),
        kind: "review".to_owned(),
        body_markdown: String::new(),
        auto_resolution_ms: None,
        questions: vec![UserInputQuestion {
            header: String::new(),
            question: "Approve the new plan?".to_owned(),
            options: vec![UserInputOption {
                label: "Approve".to_owned(),
                description: String::new(),
            }],
            multiple: false,
            allow_custom: false,
        }],
    };
    let session_id = session.id;
    let host = manager.background_handle();
    let host_join =
        tokio::spawn(async move { host.request_host_user_input(session_id, 1, request).await });

    // Wait until the tool part's operation carries a durable user-input
    // request (one host ask == one operation activity), then capture its part
    // id, its state, and the authoritative request id from the nested record.
    let (tool_part_id, request_id, state) = {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let persisted = store.load(session_id).await.expect("reload parts");
            if let Some(part) = persisted
                .parts
                .iter()
                .find(|part| tool_part_first_user_input(part).is_some())
            {
                let request_id = tool_part_first_user_input(part)
                    .expect("record present")
                    .request
                    .request_id;
                break (part.part_id, request_id, part.state);
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "host ask_user request was never recorded on the tool operation"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    };
    assert_eq!(
        state,
        PartState::InProgress,
        "the host ask_user tool part is born InProgress, not Pending — only \
         InProgress can complete through the legal in_progress -> completed edge"
    );

    // The durable presentation acknowledgement stamps presented_at into the
    // operation's request record (the guard is is_in_flight()).
    manager
        .mark_interactive_request_presented(session_id, request_id.clone())
        .await
        .expect("acknowledge presentation");
    let persisted = store.load(session_id).await.expect("reload parts");
    let presented = persisted
        .parts
        .iter()
        .find(|part| part.part_id == tool_part_id)
        .expect("tool part still present");
    assert!(
        tool_part_first_user_input(presented)
            .expect("record present")
            .request
            .presented_at
            .is_some(),
        "mark_interactive_request_presented stamps presented_at into the operation request record"
    );

    // Drive the reply. With the part born InProgress the completion is the
    // legal in_progress -> completed transition; a Pending part would be
    // rejected by the store with StoreError::InvalidState (pending -> completed
    // is not in the forward-only lifecycle, 17.2).
    let reply_request = agena_runtime::SessionExecutionReplyRequest::new(
        session_id,
        agena_runtime::SessionRunOptions {
            model: ModelRef::new("fake", "fake-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: None,
        },
        UserInputReply {
            request_id: request_id.clone(),
            kind: UserInputReplyKind::Submit,
            answers: BTreeMap::from([("0".to_owned(), vec!["Approve".to_owned()])]),
            reason: None,
        },
    );
    let _replied = manager
        .reply_user_input(reply_request)
        .await
        .expect("reply records into the operation without an invalid part transition");

    // The host call is woken through its waiter and resolves.
    let host_response = host_join
        .await
        .expect("host request task completed")
        .expect("host ask_user returned a response");
    assert!(!host_response.cancelled);
    assert_eq!(
        host_response.answers.get("0").map(Vec::as_slice),
        Some(&["Approve".to_owned()][..])
    );

    let persisted = store.load(session_id).await.expect("reload parts");
    let tool = persisted
        .parts
        .iter()
        .find(|part| part.part_id == tool_part_id)
        .expect("tool part still present");
    assert_eq!(
        tool.state,
        PartState::InProgress,
        "a host ask suspends the tool; the reply records the answer without completing the tool"
    );
    let record = tool_part_first_user_input(tool).expect("operation record still present");
    assert!(
        record.reply.is_some(),
        "the replied operation record carries the answer"
    );
    assert_eq!(
        record.reply.as_ref().expect("answered").answers.get("0"),
        Some(&vec!["Approve".to_owned()]),
        "the recorded reply matches the submitted answer"
    );
}

/// The first user-input record on a `tool_call` part's operation bucket.
fn tool_part_first_user_input(
    part: &agena_storage::store::Part,
) -> Option<agena_domain::OperationUserInputRecord> {
    if part.kind != "tool_call" {
        return None;
    }
    let content = typed_content_from_value(&part.kind, &part.content).ok()?;
    let TypedContent::ToolCall(tool_call) = content else {
        return None;
    };
    operation_from_tool_call(&tool_call)
        .user_input
        .requests
        .into_iter()
        .next()
}

/// Regression: two host `ask_user` calls from unrelated operations whose
/// provider operation id is empty (`agena.operation_id` absent — providers
/// that stream no tool-call id) must never collide in the host re-entry dedup.
/// The old key was `operation_id` + per-call sequence; with the empty id every
/// host ask in a session collapsed into the `("", 0)` bucket, so a later
/// `interaction.ask` would rediscover an earlier `plan.review` request, compare
/// its (different) questions and fail with "host user input request mismatch".
/// The dedup key is now the pending tool's durable part id, unique per
/// operation, so the two asks stay fully independent.
#[tokio::test]
async fn host_ask_user_from_unrelated_operations_with_empty_operation_id_do_not_mismatch() {
    let (manager, _database) = test_manager_with_database().await;
    let session = create(&manager, "host ask_user independence").await;

    // Two in-progress tool parts with distinct call ids and NO provider
    // operation id (`agena.operation_id` absent): `plan.set` then
    // `interaction.ask`, exactly the pair from the bug report.
    let plan_operation = agena_runtime_contracts::part::OperationPart::pending(
        1,
        ToolInvocation::new("plan.set", StructuredObject::default()),
        TimeRange {
            start_ms: 1,
            end_ms: None,
        },
    );
    let ask_operation = agena_runtime_contracts::part::OperationPart::pending(
        2,
        ToolInvocation::new("interaction.ask", StructuredObject::default()),
        TimeRange {
            start_ms: 2,
            end_ms: None,
        },
    );
    let store = manager.session_store();
    let outcome = store
        .submit_user_run(
            session.id,
            manager.store.owner_id.as_str(),
            vec![
                new_part_from_content(
                    "tool_call",
                    PartRole::Assistant,
                    &TypedContent::ToolCall(Box::new(tool_call_from_operation(&plan_operation))),
                    PartState::InProgress,
                )
                .expect("build plan tool part"),
                new_part_from_content(
                    "tool_call",
                    PartRole::Assistant,
                    &TypedContent::ToolCall(Box::new(tool_call_from_operation(&ask_operation))),
                    PartState::InProgress,
                )
                .expect("build ask tool part"),
            ],
            None,
        )
        .await
        .expect("submit run with in-progress tool parts");
    let plan_tool_part_id = outcome.parts[1].part_id;
    let ask_tool_part_id = outcome.parts[2].part_id;

    // Reload so the session projection carries both pending tools, resolvable
    // by call id exactly as `request_host_user_input` does.
    let session = manager
        .store
        .load_session(session.id)
        .await
        .expect("reload submitted run");
    assert_eq!(
        operation_id_from_part(
            session
                .part(&crate::session::model::SessionPartRef {
                    part_index: 0,
                    part_id: plan_tool_part_id,
                })
                .expect("plan tool part")
        )
        .as_deref(),
        None,
        "the test fixture really exercises the empty-operation-id case"
    );

    // A plan-review style ask for call 1, driven through the public entry
    // point in a background task (it blocks awaiting the reply, as the plugin
    // host flow does in production).
    let plan_request = crate::part::AskUserToolInput {
        title: "Approve New Plan".to_owned(),
        kind: "review".to_owned(),
        body_markdown: String::new(),
        auto_resolution_ms: None,
        questions: vec![UserInputQuestion {
            header: "Decision".to_owned(),
            question: "Approve the new plan?".to_owned(),
            options: vec![UserInputOption {
                label: "Approve".to_owned(),
                description: String::new(),
            }],
            multiple: false,
            allow_custom: false,
        }],
    };
    let session_id = session.id;
    let plan_join = tokio::spawn({
        let host = manager.background_handle();
        let request = plan_request.clone();
        async move { host.request_host_user_input(session_id, 1, request).await }
    });

    // Wait until the plan request is durable, then issue the interaction.ask
    // for call 2 with DIFFERENT questions. Under the old empty-operation-id
    // dedup this second call mismatched against the plan request; it must now
    // create its own independent request.
    let plan_request_id = wait_for_interaction_request_id(
        &store,
        session_id,
        plan_tool_part_id,
        "plan interaction part was never created",
    )
    .await;

    let ask_request = crate::part::AskUserToolInput {
        title: "Ask".to_owned(),
        kind: "ask_user".to_owned(),
        body_markdown: String::new(),
        auto_resolution_ms: None,
        questions: vec![UserInputQuestion {
            header: "Question".to_owned(),
            question: "Which flavor?".to_owned(),
            options: vec![UserInputOption {
                label: "Vanilla".to_owned(),
                description: String::new(),
            }],
            multiple: false,
            allow_custom: false,
        }],
    };
    let ask_join = tokio::spawn({
        let host = manager.background_handle();
        let request = ask_request.clone();
        async move { host.request_host_user_input(session_id, 2, request).await }
    });
    let ask_request_id = wait_for_interaction_request_id(
        &store,
        session_id,
        ask_tool_part_id,
        "ask interaction part was never created",
    )
    .await;
    assert_ne!(
        plan_request_id, ask_request_id,
        "independent host asks must own distinct request ids"
    );

    // Reply to each; each completes its own part and wakes its own host call.
    let reply = |request_id: String, answer: &str| {
        agena_runtime::SessionExecutionReplyRequest::new(
            session_id,
            agena_runtime::SessionRunOptions {
                model: ModelRef::new("fake", "fake-model"),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
            },
            UserInputReply {
                request_id,
                kind: UserInputReplyKind::Submit,
                answers: BTreeMap::from([("0".to_owned(), vec![answer.to_owned()])]),
                reason: None,
            },
        )
    };
    manager
        .reply_user_input(reply(plan_request_id.clone(), "Approve"))
        .await
        .expect("plan reply completes");
    manager
        .reply_user_input(reply(ask_request_id.clone(), "Vanilla"))
        .await
        .expect("ask reply completes");

    let plan_response = plan_join
        .await
        .expect("plan host task completed")
        .expect("plan host ask_user returned a response");
    assert_eq!(
        plan_response.answers.get("0").map(Vec::as_slice),
        Some(&["Approve".to_owned()][..]),
        "plan ask resolves with its own answer, untouched by the ask request"
    );
    let ask_response = ask_join
        .await
        .expect("ask host task completed")
        .expect("ask host ask_user returned a response");
    assert_eq!(
        ask_response.answers.get("0").map(Vec::as_slice),
        Some(&["Vanilla".to_owned()][..]),
        "interaction.ask resolves with its own answer, independent of the plan review"
    );
}

/// Poll the store until the `tool_call` operation bound to `tool_part_id`
/// carries a durable user-input request (one host ask == one operation
/// activity), returning the record's request id.
async fn wait_for_interaction_request_id(
    store: &Arc<dyn agena_storage::store::SessionStore>,
    session_id: i64,
    tool_part_id: i64,
    failure_message: &str,
) -> String {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let persisted = store.load(session_id).await.expect("reload parts");
        if let Some(request_id) = persisted.parts.iter().find_map(|part| {
            if part.kind != "tool_call" || part.part_id != tool_part_id {
                return None;
            }
            let content = typed_content_from_value(&part.kind, &part.content).ok()?;
            let TypedContent::ToolCall(tool_call) = content else {
                return None;
            };
            let operation = operation_from_tool_call(&tool_call);
            operation
                .user_input
                .requests
                .first()
                .map(|record| record.request.request_id.clone())
        }) {
            return request_id;
        }
        assert!(tokio::time::Instant::now() < deadline, "{failure_message}");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn continuation_appends_into_the_last_assistant_reply_without_a_user_run() {
    let (manager, _database) = test_manager_with_database().await;
    let session = create(&manager, "assistant continuation").await;

    // A user message, then a real assistant reply (text part under an
    // assistant `continue` run marker).
    let session = append_message(
        &manager,
        session,
        Role::User,
        vec![TypedContent::Text(text_content("user question"))],
    )
    .await;
    let reply_run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .expect("start assistant reply run");
    manager
        .store
        .append_parts(
            session.id,
            reply_run_id,
            vec![
                new_part_from_content(
                    "text",
                    PartRole::Assistant,
                    &TypedContent::Text(text_content("the answer")),
                    PartState::Completed,
                )
                .expect("build reply text part"),
            ],
        )
        .await
        .expect("append reply text part");
    // Reload so the projection carries the authoritative run marker.
    let session = manager
        .get_session(session.id)
        .await
        .expect("reload with reply marker");

    let message_count_before = parts_into_runs(session.parts()).len();
    // Inject a continuation: it must extend the assistant reply in place and
    // return `None` (no fresh marker), never fabricate a user run.
    let (continued, continuation_marker) = manager
        .inject_continuation_message(session, "keep going".to_owned())
        .await
        .expect("inject continuation");
    assert_eq!(
        continuation_marker, None,
        "reply extension reuses no run marker"
    );

    let runs = parts_into_runs(continued.parts());
    assert_eq!(
        runs.len(),
        message_count_before,
        "continuation must not add a run marker (no user-run inflation)"
    );
    let reply = runs
        .iter()
        .find(|run| run.first().is_some_and(|part| part.part_id == reply_run_id))
        .expect("the assistant reply run is present");
    let text_parts = reply
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| {
            part.content
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert!(
        text_parts.iter().any(|text| text.contains("keep going")),
        "the continuation text lands inside the assistant reply's text part: {text_parts:?}"
    );
    let last_user = continued
        .parts()
        .iter()
        .rev()
        .find(|part| part.is_run_marker() && part.role == PartRole::User);
    assert_eq!(
        last_user.map(|part| part.part_id),
        runs.first()
            .and_then(|run| run.first().map(|part| part.part_id)),
        "the user message stays the session's last user message; the continuation is not a user turn"
    );
}

#[tokio::test]
async fn continuation_without_an_assistant_reply_opens_a_fresh_continue_marker() {
    let (manager, _database) = test_manager_with_database().await;
    let session = create(&manager, "failed-turn continuation").await;
    let session = append_message(
        &manager,
        session,
        Role::User,
        vec![TypedContent::Text(text_content("user question"))],
    )
    .await;

    // No assistant reply exists: the continuation must open a fresh assistant
    // `continue` run and append the text under it.
    let (continued, continuation_marker) = manager
        .inject_continuation_message(session, "retry after failure".to_owned())
        .await
        .expect("inject continuation");
    let marker_id = continuation_marker.expect("a fresh continue marker was opened");
    let marker = continued
        .parts()
        .iter()
        .find(|part| part.part_id == marker_id)
        .expect("fresh marker present in projection");
    assert_eq!(marker.role, PartRole::Assistant);
    assert_eq!(
        marker
            .content
            .get("run_kind")
            .and_then(serde_json::Value::as_str),
        Some("continue")
    );
    let text_parts = continued
        .parts()
        .iter()
        .filter(|part| part.part_id != marker_id && part.role == PartRole::Assistant)
        .collect::<Vec<_>>();
    assert!(
        text_parts.iter().any(|part| part
            .content
            .get("text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| text.contains("retry after failure"))),
        "continuation text is recorded as an assistant part, not a user message"
    );
}

#[tokio::test]
async fn hook_runs_ride_the_launching_terminal_assistant_run() {
    use agena_plugin_host::{HookRunRecord, HookRunStatus};

    let (manager, _database) = test_manager_with_database().await;
    let session = create(&manager, "hook run").await;
    let session_id = session.id;

    // A completed final assistant reply — the launching run for hooks that
    // fire after the reply ends (agent.stop). Hook parts must be appended onto
    // it (no new run marker), keeping the AI identity.
    let run_id = manager
        .store
        .start_run(
            session_id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .expect("start launching run marker");
    manager
        .store
        .complete_run(
            session_id,
            run_id,
            agena_storage::store::RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
        )
        .await
        .expect("complete launching run");
    let session = manager
        .get_session(session_id)
        .await
        .expect("reload session with the terminal launching run");

    let runs = vec![
        HookRunRecord::new(
            "agent.stop",
            "test-plugin",
            Some(session_id),
            HookRunStatus::Applied,
            "stopped cleanly",
            None,
        )
        .with_message(Some("continue with the next plan step".to_owned())),
    ];
    let recorded = manager
        .record_hook_runs(session, runs, manager.execution_state())
        .await
        .expect("record hook runs");

    assert_eq!(
        recorded
            .parts()
            .iter()
            .filter(|part| part.is_run_marker())
            .count(),
        1,
        "hook parts ride the launching run — no new run marker"
    );
    assert!(
        !recorded.parts().iter().any(|part| part.is_run_marker()
            && part
                .content
                .get("run_kind")
                .and_then(serde_json::Value::as_str)
                == Some("execution")),
        "no execution run marker is created"
    );
    let hook_parts = recorded
        .parts()
        .iter()
        .filter(|part| part.kind == "hook")
        .collect::<Vec<_>>();
    assert_eq!(hook_parts.len(), 1, "one hook part per recorded run");
    assert_eq!(
        hook_parts[0].role,
        PartRole::Assistant,
        "hook parts keep the launching run's AI identity"
    );
    assert_eq!(
        hook_parts[0].state,
        PartState::Completed,
        "a recorded hook run is finished activity"
    );
    assert_eq!(
        hook_parts[0].run_id,
        Some(run_id),
        "the hook rides the launching run, not a new marker"
    );
    assert_eq!(
        hook_parts[0]
            .content
            .get("hook")
            .and_then(serde_json::Value::as_str),
        Some("agent.stop")
    );
    assert_eq!(
        hook_parts[0]
            .content
            .get("message")
            .and_then(serde_json::Value::as_str),
        Some("continue with the next plan step"),
        "the hook-sent continuation is carried by the hook part's message"
    );
    assert!(
        recorded
            .parts()
            .iter()
            .filter(|part| part.kind == "text" && part.role == PartRole::Assistant)
            .all(|part| part
                .content
                .get("text")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|text| !text.contains("continue with the next plan step"))),
        "the continuation is not injected as a separate assistant text part"
    );

    let state = manager
        .session_store()
        .session_state(session_id)
        .await
        .expect("derive session state");
    assert_eq!(
        state.state,
        SessionState::Ready,
        "a completed launching run plus completed hook parts leaves the session Ready, not Running"
    );
}

#[tokio::test]
async fn hook_runs_append_to_the_in_flight_launching_run() {
    use agena_plugin_host::{HookRunRecord, HookRunStatus};

    let (manager, _database) = test_manager_with_database().await;
    let session = create(&manager, "hook run").await;
    let session_id = session.id;

    // An in-flight assistant `continue` marker — the launching run for hooks
    // that fire mid-turn (a tool batch, a failed model turn). Hook parts append
    // onto it without terminalizing it or creating a new marker.
    let run_id = manager
        .store
        .start_run(
            session_id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .expect("start launching run marker");
    let session = manager
        .get_session(session_id)
        .await
        .expect("reload session with the in-flight launching run");

    let runs = vec![HookRunRecord::new(
        "command.after",
        "test-plugin",
        Some(session_id),
        HookRunStatus::Applied,
        "command.after hook ran",
        None,
    )];
    let recorded = manager
        .record_hook_runs(session, runs, manager.execution_state())
        .await
        .expect("record hook runs");

    assert_eq!(
        recorded
            .parts()
            .iter()
            .filter(|part| part.is_run_marker())
            .count(),
        1,
        "hook parts ride the in-flight launching run — no new run marker"
    );
    let marker = recorded
        .parts()
        .iter()
        .rev()
        .find(|part| part.is_run_marker())
        .expect("the launching run marker");
    assert_eq!(marker.part_id, run_id);
    assert!(
        marker.state.is_in_flight(),
        "appending hook parts to an in-flight launching run must not terminalize it"
    );
    let hook_parts = recorded
        .parts()
        .iter()
        .filter(|part| part.kind == "hook")
        .collect::<Vec<_>>();
    assert_eq!(hook_parts.len(), 1, "one hook part per recorded run");
    assert_eq!(
        hook_parts[0].role,
        PartRole::Assistant,
        "hook parts keep the launching run's AI identity"
    );
    assert_eq!(
        hook_parts[0].run_id,
        Some(run_id),
        "the hook rides the in-flight launching run"
    );
}

/// Install the durable launch receipt plus normalized Running aggregate used
/// by notification tests. The tool and its assistant run are already
/// terminal: completion/events mutate only the aggregate and append their
/// Assistant-owned hooks to this launch run.
async fn install_test_background_operation(
    manager: &SessionManager,
    session_id: i64,
    run_id: i64,
    kind: agena_storage::store::BackgroundOperationKind,
    external_id: &str,
) -> i64 {
    let mut operation = agena_runtime_contracts::part::OperationPart::pending(
        1,
        ToolInvocation::new(
            match kind {
                agena_storage::store::BackgroundOperationKind::Shell => "shell.run",
                agena_storage::store::BackgroundOperationKind::Task => "task",
                agena_storage::store::BackgroundOperationKind::Monitor => "monitor.start",
                agena_storage::store::BackgroundOperationKind::ScheduledDelivery => {
                    panic!("scheduled delivery has no launch tool")
                }
            },
            StructuredObject::default(),
        ),
        TimeRange {
            start_ms: 1,
            end_ms: Some(2),
        },
    );
    operation.set_background_operation(&agena_runtime_contracts::part::BackgroundOperation {
        kind: kind.as_str().to_owned(),
        id: external_id.to_owned(),
    });
    let tool_part = new_part_from_content(
        "tool_call",
        PartRole::Assistant,
        &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation))),
        PartState::Completed,
    )
    .expect("build completed background launch receipt");
    let created = manager
        .store
        .append_parts(session_id, run_id, vec![tool_part])
        .await
        .expect("append completed background launch receipt");
    let tool_part_id = created[0].part_id;
    manager
        .store
        .complete_run(
            session_id,
            run_id,
            agena_storage::store::RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
        )
        .await
        .expect("complete launch run");
    let operation_id = super::background_operation_id(session_id, tool_part_id);
    let created = manager
        .store
        .create_background_operation(agena_storage::store::NewBackgroundOperation {
            operation_id: operation_id.clone(),
            session_id,
            launch_run_id: Some(run_id),
            launch_tool_part_id: Some(tool_part_id),
            kind,
        })
        .await
        .expect("create background operation");
    let launching = manager
        .store
        .transition_background_operation(agena_storage::store::BackgroundOperationTransition {
            operation_id: operation_id.clone(),
            expected_revision: created.revision,
            next_phase: agena_storage::store::BackgroundOperationPhase::Launching,
            external_id: Some(external_id.to_owned()),
            outcome: None,
            failure: None,
            owner_id: Some("test-launch".to_owned()),
            lease_until_ms: Some(10_000),
        })
        .await
        .expect("transition background operation to launching");
    manager
        .store
        .transition_background_operation(agena_storage::store::BackgroundOperationTransition {
            operation_id,
            expected_revision: launching.revision,
            next_phase: agena_storage::store::BackgroundOperationPhase::Running,
            external_id: Some(external_id.to_owned()),
            outcome: None,
            failure: None,
            owner_id: None,
            lease_until_ms: None,
        })
        .await
        .expect("transition background operation to running");
    tool_part_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn assistant_hook_mid_model_part_waits_for_the_boundary_and_keeps_the_turn_continuous() {
    let provider = Arc::new(AssistantHookBoundaryProvider::new());
    let manager = Arc::new(manager_with_tool_search_fixture(provider.clone()).await);
    let session = create(&manager, "assistant hook turn continuation").await;
    let session_id = session.id;

    // Install a previously launched background operation. Its completion will
    // arrive while a newer assistant run is still inside its first provider
    // request, reproducing the same stable-loop ordering as a very fast shell
    // process whose callback beats the launch turn's follow-up model request.
    let prior_launch_run_id = manager
        .store
        .start_run(
            session_id,
            "continue",
            run_marker_content(
                "continue",
                Some("assistant-hook-boundary"),
                Some("assistant-hook-boundary-model"),
                None,
                None,
            ),
        )
        .await
        .expect("start prior background launch run");
    install_test_background_operation(
        &manager,
        session_id,
        prior_launch_run_id,
        agena_storage::store::BackgroundOperationKind::Shell,
        "proc_boundary_race",
    )
    .await;
    let operation = manager
        .store
        .background_operation_by_external_id(
            agena_storage::store::BackgroundOperationKind::Shell,
            "proc_boundary_race",
        )
        .await
        .expect("load background operation")
        .expect("background operation exists");

    let request = SessionUserRunRequest::new(
        session_id,
        agena_runtime::SessionRunOptions {
            model: ModelRef::new("assistant-hook-boundary", "assistant-hook-boundary-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: Some("Call the requested Tool API function.".to_owned()),
            temperature: Some(0.0),
            max_output_tokens: Some(256),
        },
        vec![TypedContent::Text(text_content(
            "Start the tool call, then handle any background hook.",
        ))],
    );
    let run_manager = Arc::clone(&manager);
    let execution =
        tokio::spawn(async move { run_manager.submit_subtask_user_message(request, None).await });

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        provider.first_request_entered.notified(),
    )
    .await
    .expect("first provider request entered");

    // `record_background_event` atomically appends the hook to the original
    // assistant launch run and creates the outbox delivery. The provider's
    // first response part is still blocked, so steering may only queue the
    // hook; it must not start another provider request until that part ends.
    let notification = SystemNotificationContent {
        operation_id: "proc_boundary_race".to_owned(),
        operation_kind: "shell".to_owned(),
        status: "completed".to_owned(),
        summary: "exit 0".to_owned(),
        body: "exit 0".to_owned(),
        ..Default::default()
    };
    let notification_part = new_part_from_content(
        "system_notification",
        PartRole::Assistant,
        &TypedContent::SystemNotification(notification.clone()),
        PartState::Completed,
    )
    .expect("build Assistant-owned notification");
    manager
        .store
        .record_background_event(agena_storage::store::BackgroundEventRequest {
            operation_id: operation.operation_id,
            event_key: "terminal".to_owned(),
            event_seq: None,
            next_phase: Some(agena_storage::store::BackgroundOperationPhase::Completed),
            outcome: Some(serde_json::json!({"text": "exit 0", "exit_code": 0})),
            failure: None,
            notification: notification_part,
        })
        .await
        .expect("commit terminal event and assistant hook");
    manager
        .steer_input(
            session_id,
            vec![TypedContent::SystemNotification(notification)],
        )
        .await
        .expect("wake the active stable run");
    assert_eq!(
        provider.requests().len(),
        1,
        "the hook is queued while the current provider part is still active"
    );
    provider.release_first_request.notify_one();

    let completed = tokio::time::timeout(std::time::Duration::from_secs(10), execution)
        .await
        .expect("stable run must not hang")
        .expect("stable run task joins")
        .expect("stable run completes");
    let parts = completed.parts();
    let active_launch_tool = parts
        .iter()
        .find(|part| {
            part.kind == "tool_call"
                && typed_content_from_value(&part.kind, &part.content)
                    .ok()
                    .and_then(|content| match content {
                        TypedContent::ToolCall(tool_call) => {
                            (operation_from_tool_call(&tool_call).invocation.name == "tools_search")
                                .then_some(())
                        }
                        _ => None,
                    })
                    .is_some()
        })
        .expect("the active launch turn's tool call");
    assert_eq!(active_launch_tool.state, PartState::Completed);
    let active_launch_run_id = active_launch_tool.run_id.expect("tool run id");
    let active_launch_run = parts
        .iter()
        .find(|part| part.part_id == active_launch_run_id)
        .expect("active launch run marker");
    assert_eq!(
        active_launch_run.state,
        PartState::Completed,
        "the active assistant run completes normally"
    );

    let notifications = parts
        .iter()
        .filter(|part| part.kind == "system_notification")
        .collect::<Vec<_>>();
    assert_eq!(
        notifications.len(),
        1,
        "one event produces one notification"
    );
    assert_eq!(notifications[0].role, PartRole::Assistant);
    assert_eq!(
        notifications[0].run_id,
        Some(prior_launch_run_id),
        "the hook is appended directly to the assistant turn that launched the background work"
    );
    assert!(
        !parts.iter().any(|part| {
            part.is_run_marker()
                && part.role == PartRole::Runtime
                && part
                    .content
                    .get("run_kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("runtime_ingress")
        }),
        "AI-launched hooks create no synthetic Runtime ingress turn"
    );

    let response_text = parts
        .iter()
        .find(|part| {
            part.kind == "text"
                && part.content.get("text").and_then(serde_json::Value::as_str)
                    == Some("BG_BOUNDARY_NOTIFICATION_RECEIVED")
        })
        .expect("notification response text");
    let response_run_id = response_text.run_id.expect("response run id");
    assert_eq!(
        response_run_id, active_launch_run_id,
        "the hook response remains in the already-active assistant turn"
    );
    let response_run = parts
        .iter()
        .find(|part| part.part_id == response_run_id)
        .expect("notification response run marker");
    assert_eq!(response_run.role, PartRole::Assistant);
    assert_eq!(response_run.state, PartState::Completed);
    let notification_part_id = notifications[0].part_id;
    assert!(
        response_run
            .content
            .get("rounds")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|rounds| rounds.iter().any(|round| {
                round
                    .get("input_notification_part_ids")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|ids| {
                        ids.iter()
                            .any(|id| id.as_i64() == Some(notification_part_id))
                    })
            })),
        "the follow-up provider round durably records the hook it actually saw"
    );
    assert!(
        parts.iter().all(|part| {
            !part.is_run_marker() || part.role != PartRole::Assistant || part.state.is_terminal()
        }),
        "no assistant run may remain in flight after the handoff"
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].turns.last().is_some_and(|turn| {
            matches!(
                turn.parts.as_slice(),
                [CompletionInputPart::SystemMessage { text }] if text == "exit 0"
            )
        }),
        "the queued hook is the chronological tail input to the next provider round, not retroactively inserted before the active turn"
    );
    let state = manager
        .session_store()
        .session_state(session_id)
        .await
        .expect("derive final session state");
    assert_eq!(state.state, SessionState::Ready);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn assistant_hook_after_a_terminal_text_part_opens_a_clean_follow_up_run() {
    let provider = Arc::new(AssistantHookBoundaryProvider::text_only());
    let manager = Arc::new(manager_with_tool_search_fixture(provider.clone()).await);
    let session = create(&manager, "assistant hook after terminal text part").await;
    let session_id = session.id;

    let launch_run_id = manager
        .store
        .start_run(
            session_id,
            "continue",
            run_marker_content(
                "continue",
                Some("assistant-hook-boundary"),
                Some("assistant-hook-boundary-model"),
                None,
                None,
            ),
        )
        .await
        .expect("start background launch run");
    install_test_background_operation(
        &manager,
        session_id,
        launch_run_id,
        agena_storage::store::BackgroundOperationKind::Shell,
        "proc_text_boundary",
    )
    .await;
    let operation = manager
        .store
        .background_operation_by_external_id(
            agena_storage::store::BackgroundOperationKind::Shell,
            "proc_text_boundary",
        )
        .await
        .expect("load background operation")
        .expect("background operation exists");

    let request = SessionUserRunRequest::new(
        session_id,
        agena_runtime::SessionRunOptions {
            model: ModelRef::new("assistant-hook-boundary", "assistant-hook-boundary-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: Some(0.0),
            max_output_tokens: Some(256),
        },
        vec![TypedContent::Text(text_content(
            "Finish this text part, then process the queued hook.",
        ))],
    );
    let run_manager = Arc::clone(&manager);
    let execution =
        tokio::spawn(async move { run_manager.submit_subtask_user_message(request, None).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        provider.first_request_entered.notified(),
    )
    .await
    .expect("first provider request entered");

    let notification = SystemNotificationContent {
        operation_id: "proc_text_boundary".to_owned(),
        operation_kind: "shell".to_owned(),
        status: "completed".to_owned(),
        summary: "exit 0".to_owned(),
        body: "exit 0".to_owned(),
        ..Default::default()
    };
    let settled = manager
        .store
        .record_background_event(agena_storage::store::BackgroundEventRequest {
            operation_id: operation.operation_id,
            event_key: "terminal".to_owned(),
            event_seq: None,
            next_phase: Some(agena_storage::store::BackgroundOperationPhase::Completed),
            outcome: Some(serde_json::json!({"text": "exit 0"})),
            failure: None,
            notification: new_part_from_content(
                "system_notification",
                PartRole::Assistant,
                &TypedContent::SystemNotification(notification.clone()),
                PartState::Completed,
            )
            .expect("build assistant hook"),
        })
        .await
        .expect("commit assistant hook");
    manager
        .steer_input(
            session_id,
            vec![TypedContent::SystemNotification(notification)],
        )
        .await
        .expect("queue assistant hook");
    assert_eq!(
        provider.requests().len(),
        1,
        "the hook cannot interrupt the active text part"
    );
    provider.release_first_request.notify_one();

    let completed = tokio::time::timeout(std::time::Duration::from_secs(10), execution)
        .await
        .expect("stable run must not hang")
        .expect("stable run task joins")
        .expect("stable run completes");
    let first_text = completed
        .parts()
        .iter()
        .find(|part| {
            part.kind == "text"
                && part.content.get("text").and_then(serde_json::Value::as_str)
                    == Some("FIRST_PROVIDER_PART_COMPLETED")
        })
        .expect("first terminal provider text");
    let response_text = completed
        .parts()
        .iter()
        .find(|part| {
            part.kind == "text"
                && part.content.get("text").and_then(serde_json::Value::as_str)
                    == Some("BG_BOUNDARY_NOTIFICATION_RECEIVED")
        })
        .expect("hook response text");
    assert_ne!(
        first_text.run_id, response_text.run_id,
        "a terminal marker is immutable, so the queued hook opens one clean Assistant follow-up run"
    );
    assert_eq!(
        settled.notification_part.role,
        PartRole::Assistant,
        "the hook keeps AI identity"
    );
    assert_eq!(settled.notification_part.run_id, Some(launch_run_id));
    assert!(
        completed.parts().iter().all(|part| {
            !part.is_run_marker() || part.role != PartRole::Assistant || part.state.is_terminal()
        }),
        "both the completed text run and hook follow-up run are terminal"
    );
    assert_eq!(provider.requests().len(), 2);
    assert!(
        manager
            .notification_has_completed_assistant_response(
                session_id,
                settled.delivery.notification_part_id,
            )
            .await
            .expect("derive exact provider-round receipt")
    );
}

#[tokio::test]
async fn background_completion_notification_is_committed_once_per_operation() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged the finished background operation".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session =
        create_with_model(&manager, "background notification", "fake", "fake-model").await;
    let session_id = session.id;

    // A completed launch receipt. The launched process has its own durable
    // Running lifecycle and is not represented by an in-progress tool part.
    let run_id = manager
        .store
        .start_run(
            session_id,
            "continue",
            run_marker_content("continue", Some("fake"), Some("fake-model"), None, None),
        )
        .await
        .expect("start launching run marker");
    let tool_part_id = install_test_background_operation(
        &manager,
        session_id,
        run_id,
        agena_storage::store::BackgroundOperationKind::Shell,
        "proc_test",
    )
    .await;

    let notification = SystemNotificationContent {
        operation_id: "proc_test".to_string(),
        operation_kind: "shell".to_string(),
        status: "completed".to_string(),
        summary: "exit 0".to_string(),
        body: "exit 0".to_string(),
        ..Default::default()
    };

    // First delivery atomically terminalizes the aggregate, appends an
    // Assistant-owned hook to the launch turn, and enqueues the durable wake
    // delivery.
    manager
        .settle_background_operation(
            session_id,
            "shell",
            "proc_test",
            PartState::Completed,
            Ok("exit 0".to_owned()),
            notification.clone(),
        )
        .await
        .expect("settle background operation");
    let reloaded = manager
        .get_session(session_id)
        .await
        .expect("reload session");
    let parts = reloaded.parts();
    let notified = parts
        .iter()
        .filter(|part| part.kind == "system_notification")
        .collect::<Vec<_>>();
    assert_eq!(notified.len(), 1, "exactly one notification part");
    assert_eq!(
        notified[0].role,
        PartRole::Assistant,
        "AI-launched work keeps the launching assistant identity"
    );
    assert_eq!(
        notified[0].run_id,
        Some(run_id),
        "the notification is a rich hook part on the existing launch turn"
    );

    let launching_marker = parts
        .iter()
        .find(|part| part.is_run_marker() && part.part_id == run_id)
        .expect("launching run marker");
    assert!(
        launching_marker.state.is_terminal(),
        "the launch run remains terminal and immutable"
    );

    // The launching tool part was terminalized by the settle.
    let tool = parts
        .iter()
        .find(|part| part.part_id == tool_part_id)
        .expect("tool part");
    assert!(tool.state.is_terminal(), "launch receipt remains terminal");

    // A re-delivered completion signal is a no-op: the durable part is the claim.
    manager
        .settle_background_operation(
            session_id,
            "shell",
            "proc_test",
            PartState::Completed,
            Ok("exit 0".to_owned()),
            notification.clone(),
        )
        .await
        .expect("re-settle background operation");
    let reloaded = manager
        .get_session(session_id)
        .await
        .expect("reload session after dedup");
    assert_eq!(
        reloaded
            .parts()
            .iter()
            .filter(|part| part.kind == "system_notification")
            .count(),
        1,
        "dedup keeps a single notification part"
    );
}

#[tokio::test]
async fn terminal_task_is_reconciled_from_durable_child_state_without_observer_event() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged the reconciled task".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let parent = create_with_model(
        &manager,
        "durable task reconciliation",
        "fake",
        "fake-model",
    )
    .await;
    let run_id = manager
        .store
        .start_run(
            parent.id,
            "continue",
            run_marker_content("continue", Some("fake"), Some("fake-model"), None, None),
        )
        .await
        .expect("start task launch run");
    install_test_background_operation(
        &manager,
        parent.id,
        run_id,
        agena_storage::store::BackgroundOperationKind::Task,
        "task_reconcile_durable",
    )
    .await;
    let child_id = manager
        .store
        .create_subagent_session(
            parent.id,
            "task_reconcile_durable".to_owned(),
            "durable child".to_owned(),
        )
        .await
        .expect("create child session");
    let mut child = manager
        .store
        .load_session(child_id)
        .await
        .expect("load child session");
    child.runtime.subtask.status = agena_domain::SubtaskStatus::Completed;
    child.runtime.subtask.started_at_ms = Some(10);
    child.runtime.subtask.finished_at_ms = Some(20);
    manager
        .store
        .update_subtask_state(
            child,
            Some("completed".to_owned()),
            Some(10),
            Some(20),
            None,
        )
        .await
        .expect("persist terminal child state");

    let reconciled = manager
        .reconcile_background_tasks(16)
        .await
        .expect("reconcile task aggregate");
    assert_eq!(reconciled, 1);
    let operation = manager
        .store
        .background_operation_by_external_id(
            agena_storage::store::BackgroundOperationKind::Task,
            "task_reconcile_durable",
        )
        .await
        .expect("load task operation")
        .expect("task operation exists");
    assert_eq!(
        operation.phase,
        agena_storage::store::BackgroundOperationPhase::Completed
    );
    let reloaded = manager
        .get_session(parent.id)
        .await
        .expect("reload reconciled parent");
    assert_eq!(
        reloaded
            .parts()
            .iter()
            .filter(|part| {
                part.kind == "system_notification" && part.role == PartRole::Assistant
            })
            .count(),
        1,
        "durable reconciliation emits exactly one Assistant-owned notification"
    );
    assert_eq!(
        manager
            .reconcile_background_tasks(16)
            .await
            .expect("repeat reconciliation"),
        0,
        "terminal aggregate drops out of the active reconciliation scan"
    );
}

#[tokio::test]
async fn committed_notification_is_woken_and_consumed_after_dispatcher_restart_recovery() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged recovered delivery".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session =
        create_with_model(&manager, "delivery crash recovery", "fake", "fake-model").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", Some("fake"), Some("fake-model"), None, None),
        )
        .await
        .expect("start launch run");
    install_test_background_operation(
        &manager,
        session.id,
        run_id,
        agena_storage::store::BackgroundOperationKind::Shell,
        "proc_recover_delivery",
    )
    .await;
    let operation = manager
        .store
        .background_operation_by_external_id(
            agena_storage::store::BackgroundOperationKind::Shell,
            "proc_recover_delivery",
        )
        .await
        .expect("load operation")
        .expect("operation exists");
    let notification = SystemNotificationContent {
        operation_id: "proc_recover_delivery".to_owned(),
        operation_kind: "shell".to_owned(),
        status: "completed".to_owned(),
        summary: "recovered completion".to_owned(),
        body: "recovered completion".to_owned(),
        ..Default::default()
    };
    let notification_part = new_part_from_content(
        "system_notification",
        PartRole::Assistant,
        &TypedContent::SystemNotification(notification),
        PartState::Completed,
    )
    .expect("build notification");
    manager
        .store
        .record_background_event(agena_storage::store::BackgroundEventRequest {
            operation_id: operation.operation_id,
            event_key: "terminal".to_owned(),
            event_seq: None,
            next_phase: Some(agena_storage::store::BackgroundOperationPhase::Completed),
            outcome: Some(serde_json::json!({"text": "done"})),
            failure: None,
            notification: notification_part,
        })
        .await
        .expect("commit event and outbox without dispatch");
    assert_eq!(
        manager
            .store
            .pending_background_deliveries(16)
            .await
            .expect("pending before recovery")
            .len(),
        1,
        "fixture models a crash after commit and before wake"
    );

    assert_eq!(
        manager
            .recover_background_deliveries(16)
            .await
            .expect("recover committed delivery"),
        1
    );
    assert!(
        manager
            .store
            .pending_background_deliveries(16)
            .await
            .expect("pending after recovery")
            .is_empty(),
        "wake success consumes the durable delivery"
    );
}

#[tokio::test]
async fn non_retryable_background_wake_is_terminalized_instead_of_requeued_forever() {
    let manager = manager_with_provider(Arc::new(StartupFailureProvider {
        model: ModelId::new("failure-model"),
    }))
    .await;
    let session = create_with_model(
        &manager,
        "non-retryable delivery",
        "startup-failure",
        "failure-model",
    )
    .await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content(
                "continue",
                Some("startup-failure"),
                Some("failure-model"),
                None,
                None,
            ),
        )
        .await
        .expect("start launch run");
    install_test_background_operation(
        &manager,
        session.id,
        run_id,
        agena_storage::store::BackgroundOperationKind::Shell,
        "proc_non_retryable_delivery",
    )
    .await;

    let notification = SystemNotificationContent {
        operation_id: "proc_non_retryable_delivery".to_owned(),
        operation_kind: "shell".to_owned(),
        status: "completed".to_owned(),
        summary: "exit 0".to_owned(),
        body: "exit 0".to_owned(),
        ..Default::default()
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        manager.settle_background_operation(
            session.id,
            "shell",
            "proc_non_retryable_delivery",
            PartState::Completed,
            Ok("exit 0".to_owned()),
            notification,
        ),
    )
    .await
    .expect("non-retryable wake must finish promptly")
    .expect("delivery settlement succeeds after terminalizing its failed wake");

    assert!(
        manager
            .store
            .pending_background_deliveries(16)
            .await
            .expect("read pending deliveries")
            .is_empty(),
        "a non-retryable wake must not be returned to pending"
    );
    let part_count = manager
        .store
        .load_session(session.id)
        .await
        .expect("load failed delivery transcript")
        .parts()
        .len();
    assert_eq!(
        manager
            .recover_background_deliveries(16)
            .await
            .expect("repeat recovery"),
        0,
        "failed delivery is not selected by recovery"
    );
    assert_eq!(
        manager
            .store
            .load_session(session.id)
            .await
            .expect("reload failed delivery transcript")
            .parts()
            .len(),
        part_count,
        "repeat recovery must not create another model execution"
    );
}

#[tokio::test]
async fn recovery_consumes_a_delivery_whose_response_committed_before_the_outbox_ack() {
    // If recovery invokes this provider, the test fails: the transcript already
    // contains durable proof that the notification response committed before
    // the original dispatcher crashed.
    let provider = Arc::new(StartupFailureProvider {
        model: ModelId::new("startup-failure-model"),
    });
    let manager = manager_with_provider(provider).await;
    let session = create_with_model(
        &manager,
        "post-response delivery recovery",
        "startup-failure",
        "startup-failure-model",
    )
    .await;
    let launch_run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content(
                "continue",
                Some("startup-failure"),
                Some("startup-failure-model"),
                None,
                None,
            ),
        )
        .await
        .expect("start launch run");
    install_test_background_operation(
        &manager,
        session.id,
        launch_run_id,
        agena_storage::store::BackgroundOperationKind::Shell,
        "proc_response_committed",
    )
    .await;
    let operation = manager
        .store
        .background_operation_by_external_id(
            agena_storage::store::BackgroundOperationKind::Shell,
            "proc_response_committed",
        )
        .await
        .expect("load operation")
        .expect("operation exists");
    let notification = SystemNotificationContent {
        operation_id: "proc_response_committed".to_owned(),
        operation_kind: "shell".to_owned(),
        status: "completed".to_owned(),
        summary: "exit 0".to_owned(),
        body: "exit 0".to_owned(),
        ..Default::default()
    };
    let settled = manager
        .store
        .record_background_event(agena_storage::store::BackgroundEventRequest {
            operation_id: operation.operation_id,
            event_key: "terminal".to_owned(),
            event_seq: None,
            next_phase: Some(agena_storage::store::BackgroundOperationPhase::Completed),
            outcome: Some(serde_json::json!({"text": "exit 0"})),
            failure: None,
            notification: new_part_from_content(
                "system_notification",
                PartRole::Assistant,
                &TypedContent::SystemNotification(notification),
                PartState::Completed,
            )
            .expect("build notification"),
        })
        .await
        .expect("commit notification and pending delivery");

    // Model the original dispatcher completing its model wake and crashing in
    // the tiny window before `consume_background_delivery` commits.
    let response_run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content(
                "continue",
                Some("startup-failure"),
                Some("startup-failure-model"),
                None,
                None,
            ),
        )
        .await
        .expect("start committed response run");
    manager
        .store
        .append_parts(
            session.id,
            response_run_id,
            vec![
                new_part_from_content(
                    "text",
                    PartRole::Assistant,
                    &TypedContent::Text(text_content("already acknowledged".to_owned())),
                    PartState::Completed,
                )
                .expect("build response text"),
            ],
        )
        .await
        .expect("append committed response");
    let notification_part_id = settled
        .delivery
        .notification_part_id
        .expect("settled delivery notification id");
    let mut response_marker_content = run_marker_content(
        "continue",
        Some("startup-failure"),
        Some("startup-failure-model"),
        None,
        None,
    );
    response_marker_content["rounds"] = serde_json::json!([{
        "part_ids": [],
        "provider_state": null,
        "input_notification_part_ids": [notification_part_id],
    }]);
    manager
        .store
        .complete_run(
            session.id,
            response_run_id,
            agena_storage::store::RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: Some(response_marker_content),
                provider_state: None,
            },
        )
        .await
        .expect("complete response run");
    assert!(
        manager
            .notification_has_completed_assistant_response(session.id, Some(notification_part_id),)
            .await
            .expect("derive response evidence")
    );
    let assistant_runs_before = manager
        .get_session(session.id)
        .await
        .expect("load before recovery")
        .parts()
        .iter()
        .filter(|part| part.is_run_marker() && part.role == PartRole::Assistant)
        .count();

    assert_eq!(
        manager
            .recover_background_deliveries(16)
            .await
            .expect("recover post-response delivery"),
        1
    );
    assert!(
        manager
            .store
            .pending_background_deliveries(16)
            .await
            .expect("pending after recovery")
            .is_empty(),
        "recovery consumes the delivery after finding durable response evidence"
    );
    let assistant_runs_after = manager
        .get_session(session.id)
        .await
        .expect("load after recovery")
        .parts()
        .iter()
        .filter(|part| part.is_run_marker() && part.role == PartRole::Assistant)
        .count();
    assert_eq!(
        assistant_runs_after, assistant_runs_before,
        "recovery must not invoke the model and create a duplicate response"
    );
}

#[tokio::test]
async fn restarted_running_task_without_live_child_lease_becomes_interrupted() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged interrupted task".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let parent =
        create_with_model(&manager, "interrupted task recovery", "fake", "fake-model").await;
    let run_id = manager
        .store
        .start_run(
            parent.id,
            "continue",
            run_marker_content("continue", Some("fake"), Some("fake-model"), None, None),
        )
        .await
        .expect("start task launch run");
    install_test_background_operation(
        &manager,
        parent.id,
        run_id,
        agena_storage::store::BackgroundOperationKind::Task,
        "task_restart_orphan",
    )
    .await;
    let child_id = manager
        .store
        .create_subagent_session(
            parent.id,
            "task_restart_orphan".to_owned(),
            "orphan child".to_owned(),
        )
        .await
        .expect("create child");
    let child = manager
        .store
        .load_session(child_id)
        .await
        .expect("load child");
    manager
        .store
        .update_subtask_state(child, Some("running".to_owned()), Some(10), None, None)
        .await
        .expect("persist stale Running child");

    assert_eq!(
        manager
            .reconcile_background_tasks(16)
            .await
            .expect("reconcile orphan task"),
        1
    );
    let operation = manager
        .store
        .background_operation_by_external_id(
            agena_storage::store::BackgroundOperationKind::Task,
            "task_restart_orphan",
        )
        .await
        .expect("load operation")
        .expect("operation exists");
    assert_eq!(
        operation.phase,
        agena_storage::store::BackgroundOperationPhase::Interrupted
    );
    let child = manager
        .store
        .load_session(child_id)
        .await
        .expect("reload child");
    assert_eq!(
        child.runtime.subtask.status,
        agena_domain::SubtaskStatus::Interrupted
    );
}

#[tokio::test]
async fn expired_process_owner_without_registry_entry_becomes_interrupted() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged interrupted process".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session =
        create_with_model(&manager, "orphan process recovery", "fake", "fake-model").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", Some("fake"), Some("fake-model"), None, None),
        )
        .await
        .expect("start shell launch run");
    install_test_background_operation(
        &manager,
        session.id,
        run_id,
        agena_storage::store::BackgroundOperationKind::Shell,
        "proc_restart_orphan",
    )
    .await;

    assert_eq!(
        manager
            .reconcile_background_processes(16)
            .await
            .expect("reconcile orphan process"),
        1
    );
    let operation = manager
        .store
        .background_operation_by_external_id(
            agena_storage::store::BackgroundOperationKind::Shell,
            "proc_restart_orphan",
        )
        .await
        .expect("load process operation")
        .expect("process operation exists");
    assert_eq!(
        operation.phase,
        agena_storage::store::BackgroundOperationPhase::Interrupted
    );
}

#[tokio::test]
async fn background_launch_receipt_is_terminal_and_needs_no_guard() {
    let (manager, database) = test_manager_with_database().await;
    let session = create(&manager, "durable background launch").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .expect("start launching run");
    let invocation = ToolInvocation::new(
        "shell.run",
        StructuredObject::try_from(serde_json::json!({"run_in_background": true}))
            .expect("structured shell input"),
    );
    let operation = agena_runtime_contracts::part::OperationPart::pending(
        41,
        invocation,
        TimeRange {
            start_ms: 1,
            end_ms: None,
        },
    );
    let mut tool_part = new_part_from_content(
        "tool_call",
        PartRole::Assistant,
        &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation))),
        PartState::InProgress,
    )
    .expect("build launching tool part");
    tool_part.summary = Some("Background shell".to_owned());
    let created = manager
        .store
        .append_parts(session.id, run_id, vec![tool_part])
        .await
        .expect("append launching tool part");
    let tool_part_id = created[0].part_id;
    let session = manager
        .store
        .load_session(session.id)
        .await
        .expect("load pending background tool");
    let pending = session
        .pending_tool_by_part_id(tool_part_id)
        .expect("resolve pending shell tool");
    let output = crate::tool::ToolPayloadOutput::Shell {
        action: "run".to_owned(),
        shell: Some(agena_domain::ProcessShell::Bash),
        background: true,
        process_id: Some("proc_durable_launch".to_owned()),
        status: Some(agena_domain::ProcessStatus::Running),
        output: None,
        description: None,
        events: Vec::new(),
        processes: Vec::new(),
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        exit_code: None,
    }
    .into_tool_output();
    let execution = crate::tool::ToolInvocationExecution::new(
        output,
        crate::tool::ToolExecutionView::simple(
            "Run process",
            "Background · running",
            "Started background process proc_durable_launch",
        ),
    );

    manager
        .apply_tool_execution_result(session, &pending, Ok(execution), manager.execution_state())
        .await
        .expect("commit background launch");

    // Bypass the facade's streaming overlay and inspect SQLite directly. The
    // launch receipt is a normal completed tool call; external lifecycle state
    // lives in the normalized background-operation aggregate.
    let row = database
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT state, content, finished_at_ms FROM agena_parts WHERE part_id = ?",
            [tool_part_id.into()],
        ))
        .await
        .expect("query durable tool row")
        .expect("durable tool row exists");
    let state: String = row.try_get("", "state").expect("read state");
    let content: String = row.try_get("", "content").expect("read content");
    let finished_at_ms: Option<i64> = row
        .try_get("", "finished_at_ms")
        .expect("read finish timestamp");
    let content: serde_json::Value = serde_json::from_str(&content).expect("decode tool content");
    assert_eq!(state, "completed");
    assert!(finished_at_ms.is_some());
    assert_eq!(
        content["metadata"]["agena.background"]["id"],
        "proc_durable_launch"
    );

    let guard_count = database
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM agena_parts \
             WHERE kind = 'tool_result' AND parent_part_id = ? AND state = 'completed'",
            [tool_part_id.into()],
        ))
        .await
        .expect("query durable guard")
        .expect("guard count row");
    let guard_count: i64 = guard_count.try_get("", "count").expect("read guard count");
    assert_eq!(
        guard_count, 0,
        "no synthetic tool-result guard is committed"
    );
}

#[tokio::test]
async fn monitor_events_append_assistant_hooks_without_terminalizing_the_operation() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged the monitor event".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session =
        create_with_model(&manager, "monitor event notification", "fake", "fake-model").await;
    let session_id = session.id;

    // A completed monitor launch receipt backed by a Running aggregate.
    let run_id = manager
        .store
        .start_run(
            session_id,
            "continue",
            run_marker_content("continue", Some("fake"), Some("fake-model"), None, None),
        )
        .await
        .expect("start launching run marker");
    let tool_part_id = install_test_background_operation(
        &manager,
        session_id,
        run_id,
        agena_storage::store::BackgroundOperationKind::Monitor,
        "monitor_test",
    )
    .await;

    let event = |seq: u64| SystemNotificationContent {
        operation_id: "monitor_test".to_string(),
        operation_kind: "monitor".to_string(),
        status: "event".to_string(),
        summary: format!("#{seq:>5} out line-{seq}"),
        body: format!("#{seq:>5} out line-{seq}"),
        event_seq: Some(seq),
        ..Default::default()
    };

    // First event: a unique Assistant-owned hook is appended to the launch
    // turn, while the normalized monitor aggregate remains Running.
    manager
        .settle_background_event(session_id, "monitor", "monitor_test", 1, event(1))
        .await
        .expect("settle monitor event 1");
    let reloaded = manager
        .get_session(session_id)
        .await
        .expect("reload session");
    let notified = reloaded
        .parts()
        .iter()
        .filter(|part| part.kind == "system_notification")
        .collect::<Vec<_>>();
    assert_eq!(notified.len(), 1, "exactly one event part");
    assert_eq!(
        notified[0].role,
        PartRole::Assistant,
        "the event keeps its AI launch identity"
    );
    assert_eq!(
        notified[0].run_id,
        Some(run_id),
        "event appends directly to the monitor launch turn"
    );
    let tool = reloaded
        .parts()
        .iter()
        .find(|part| part.part_id == tool_part_id)
        .expect("monitor tool part");
    assert_eq!(
        tool.state,
        PartState::Completed,
        "the launch receipt remains completed after an event"
    );
    let launching_marker = reloaded
        .parts()
        .iter()
        .find(|part| part.is_run_marker() && part.part_id == run_id)
        .expect("launching run marker");
    assert!(
        launching_marker.state.is_terminal(),
        "an event never reopens or mutates the launch run"
    );

    // Each event is independently notified (per-event claim, monotonic seq).
    manager
        .settle_background_event(session_id, "monitor", "monitor_test", 2, event(2))
        .await
        .expect("settle monitor event 2");
    manager
        .settle_background_event(session_id, "monitor", "monitor_test", 2, event(2))
        .await
        .expect("re-delivered event 2");
    let reloaded = manager
        .get_session(session_id)
        .await
        .expect("reload session");
    let event_count = reloaded
        .parts()
        .iter()
        .filter(|part| part.kind == "system_notification")
        .count();
    assert_eq!(event_count, 2, "each event committed once, dedup by seq");

    // The terminal settle is not swallowed by event rows: it terminalizes the
    // aggregate and appends its own terminal Assistant hook.
    let terminal = SystemNotificationContent {
        operation_id: "monitor_test".to_string(),
        operation_kind: "monitor".to_string(),
        status: "completed".to_string(),
        summary: "Monitor finished".to_string(),
        body: "Monitor finished".to_string(),
        ..Default::default()
    };
    manager
        .settle_background_operation(
            session_id,
            "monitor",
            "monitor_test",
            PartState::Completed,
            Ok("Monitor finished".to_owned()),
            terminal,
        )
        .await
        .expect("settle monitor terminal");
    let reloaded = manager
        .get_session(session_id)
        .await
        .expect("reload session");
    let notifications = reloaded
        .parts()
        .iter()
        .filter(|part| part.kind == "system_notification")
        .collect::<Vec<_>>();
    assert_eq!(
        notifications.len(),
        3,
        "two events plus the terminal completion"
    );
    assert!(
        notifications.iter().any(|part| {
            part.role == PartRole::Assistant
                && part.run_id == Some(run_id)
                && typed_content_from_value(&part.kind, &part.content)
                    .ok()
                    .and_then(|content| match content {
                        TypedContent::SystemNotification(notification) => {
                            (notification.event_seq.is_none()
                                && notification.operation_id == "monitor_test")
                                .then_some(())
                        }
                        _ => None,
                    })
                    .is_some()
        }),
        "the terminal Assistant notification is distinct from sequenced events"
    );
    let tool = reloaded
        .parts()
        .iter()
        .find(|part| part.part_id == tool_part_id)
        .expect("monitor tool part");
    assert!(
        tool.state.is_terminal(),
        "the launch receipt remains terminal"
    );
    let launching_marker = reloaded
        .parts()
        .iter()
        .find(|part| part.is_run_marker() && part.part_id == run_id)
        .expect("launching run marker");
    assert!(
        launching_marker.state.is_terminal(),
        "the launch run remains terminal and unchanged"
    );
}

#[tokio::test]
async fn assistant_created_scheduled_deliveries_reuse_the_launch_run_without_runtime_ingress() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged assistant schedule".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session = create_with_model(
        &manager,
        "assistant scheduled delivery",
        "fake",
        "fake-model",
    )
    .await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .expect("start launch run");
    let mut tool_part = NewPart::pending(
        "tool_call",
        PartRole::Assistant,
        serde_json::json!({"operation": {"call_id": 77, "title": "cron.create"}}),
    );
    tool_part.state = PartState::Completed;
    let tool_part_id = manager
        .store
        .append_parts(session.id, run_id, vec![tool_part])
        .await
        .expect("append cron receipt")[0]
        .part_id;
    manager
        .store
        .complete_run(
            session.id,
            run_id,
            agena_storage::store::RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
        )
        .await
        .expect("complete launch run");
    let provenance = agena_scheduler::ScheduledJobLaunchProvenance {
        session_id: session.id,
        run_id,
        tool_part_id,
        call_id: 77,
    };

    for delivery_key in ["assistant-fire-1", "assistant-fire-2"] {
        assert!(
            manager
                .deliver_scheduled_job(
                    session.id,
                    "job-assistant".to_owned(),
                    delivery_key.to_owned(),
                    format!("wake for {delivery_key}"),
                    Some(provenance),
                )
                .await
                .expect("deliver assistant schedule")
        );
    }

    let reloaded = manager
        .get_session(session.id)
        .await
        .expect("reload scheduled session");
    let notifications = reloaded
        .parts()
        .iter()
        .filter(|part| part.kind == "system_notification")
        .collect::<Vec<_>>();
    assert_eq!(notifications.len(), 2);
    assert!(
        notifications
            .iter()
            .all(|part| part.role == PartRole::Assistant && part.run_id == Some(run_id))
    );
    assert!(
        !reloaded
            .parts()
            .iter()
            .any(|part| part.is_run_marker() && part.role == PartRole::Runtime)
    );
    for delivery_key in ["assistant-fire-1", "assistant-fire-2"] {
        let operation = manager
            .store
            .background_operation(&format!("scheduled:{}:{delivery_key}", session.id))
            .await
            .expect("load scheduled operation")
            .expect("scheduled operation exists");
        assert_eq!(operation.launch_run_id, Some(run_id));
        assert_eq!(operation.launch_tool_part_id, Some(tool_part_id));
    }
}

#[tokio::test]
async fn scheduled_delivery_creates_a_chronological_runtime_ingress() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged the scheduled job".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session = create_with_model(&manager, "scheduled delivery", "fake", "fake-model").await;
    let session_id = session.id;

    // A completed final assistant reply already exists. The scheduled prompt
    // must arrive after it, never be retroactively attached beneath it.
    let run_id = manager
        .store
        .start_run(
            session_id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .expect("start launching run marker");
    manager
        .store
        .complete_run(
            session_id,
            run_id,
            agena_storage::store::RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
        )
        .await
        .expect("complete launching run");

    // First delivery creates a Runtime ingress and durable outbox claim.
    let delivered = manager
        .deliver_scheduled_job(
            session_id,
            "job-1".to_owned(),
            "delivery-key-1".to_owned(),
            "check the background task list and report".to_owned(),
            None,
        )
        .await
        .expect("deliver scheduled job");
    assert!(delivered, "first delivery is new");
    let reloaded = manager
        .get_session(session_id)
        .await
        .expect("reload session");
    let parts = reloaded.parts();
    let notifications = parts
        .iter()
        .filter(|part| part.kind == "system_notification")
        .collect::<Vec<_>>();
    assert_eq!(
        notifications.len(),
        1,
        "exactly one scheduled delivery notification"
    );
    assert_eq!(
        notifications[0].role,
        PartRole::Runtime,
        "the delivery has explicit Runtime identity"
    );
    assert_ne!(
        notifications[0].run_id,
        Some(run_id),
        "the delivery is later than and independent from the assistant run"
    );
    let ingress = parts
        .iter()
        .find(|part| part.is_run_marker() && part.part_id == notifications[0].run_id.unwrap())
        .expect("scheduled Runtime ingress marker");
    assert_eq!(ingress.role, PartRole::Runtime);
    let content = typed_content_from_value(&notifications[0].kind, &notifications[0].content)
        .expect("decode");
    let TypedContent::SystemNotification(notification) = content else {
        panic!("delivery part is a system_notification");
    };
    assert_eq!(notification.operation_kind, "scheduled_delivery");
    assert_eq!(notification.operation_id, "delivery-key-1");
    assert_eq!(notification.status, "submitted");
    assert_eq!(
        notification.body, "check the background task list and report",
        "the job prompt is the model-visible body"
    );
    assert!(
        !parts.iter().any(|part| part.is_run_marker()
            && part.role == PartRole::User
            && part
                .content
                .get("run_kind")
                .and_then(serde_json::Value::as_str)
                == Some("user_send")),
        "no user_send run is created for a scheduled delivery"
    );
    let state = manager
        .session_store()
        .session_state(session_id)
        .await
        .expect("derive session state");
    assert_eq!(
        state.state,
        SessionState::Ready,
        "the delivery leaves the session Ready once the wake turn finishes"
    );

    // A re-delivered job is a no-op: the durable part is the claim.
    let redelivered = manager
        .deliver_scheduled_job(
            session_id,
            "job-1".to_owned(),
            "delivery-key-1".to_owned(),
            "check the background task list and report".to_owned(),
            None,
        )
        .await
        .expect("re-deliver scheduled job");
    assert!(!redelivered, "re-delivery is a no-op");
    let reloaded = manager
        .get_session(session_id)
        .await
        .expect("reload session after dedup");
    assert_eq!(
        reloaded
            .parts()
            .iter()
            .filter(|part| part.kind == "system_notification")
            .count(),
        1,
        "dedup keeps a single delivery part"
    );
}

#[tokio::test]
async fn scheduled_delivery_works_in_a_session_with_no_prior_run() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged the scheduled job".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session = create_with_model(
        &manager,
        "scheduled delivery empty session",
        "fake",
        "fake-model",
    )
    .await;
    let session_id = session.id;

    let delivered = manager
        .deliver_scheduled_job(
            session_id,
            "job-2".to_owned(),
            "delivery-key-2".to_owned(),
            "summarize what changed".to_owned(),
            None,
        )
        .await
        .expect("deliver scheduled job");
    assert!(delivered, "first delivery is new");
    let reloaded = manager
        .get_session(session_id)
        .await
        .expect("reload session");
    let parts = reloaded.parts();
    let notifications = parts
        .iter()
        .filter(|part| part.kind == "system_notification")
        .collect::<Vec<_>>();
    assert_eq!(notifications.len(), 1, "exactly one delivery part");
    assert_eq!(notifications[0].role, PartRole::Runtime);
    let ingress = parts
        .iter()
        .find(|part| part.is_run_marker() && part.part_id == notifications[0].run_id.unwrap())
        .expect("scheduled Runtime ingress marker");
    assert_eq!(ingress.role, PartRole::Runtime);
    assert!(
        !parts.iter().any(|part| part.is_run_marker()
            && part.role == PartRole::User
            && part
                .content
                .get("run_kind")
                .and_then(serde_json::Value::as_str)
                == Some("user_send")),
        "no user_send run is created for a scheduled delivery"
    );
    assert!(
        parts
            .iter()
            .any(|part| part.is_run_marker() && part.role == PartRole::Assistant),
        "the wake response may create an assistant run, but delivery itself remains Runtime"
    );
}

#[tokio::test]
async fn concurrent_scheduled_redelivery_commits_one_operation_event() {
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged concurrent schedule".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = Arc::new(manager_with_provider(provider).await);
    let session = create_with_model(
        &manager,
        "concurrent scheduled delivery",
        "fake",
        "fake-model",
    )
    .await;
    let left = Arc::clone(&manager);
    let right = Arc::clone(&manager);
    let (first, second) = tokio::join!(
        left.deliver_scheduled_job(
            session.id,
            "job-concurrent".to_owned(),
            "delivery-concurrent".to_owned(),
            "run once".to_owned(),
            None,
        ),
        right.deliver_scheduled_job(
            session.id,
            "job-concurrent".to_owned(),
            "delivery-concurrent".to_owned(),
            "run once".to_owned(),
            None,
        ),
    );
    let created = [
        first.expect("first delivery"),
        second.expect("second delivery"),
    ];
    assert_eq!(created.into_iter().filter(|created| *created).count(), 1);
    let reloaded = manager
        .get_session(session.id)
        .await
        .expect("reload scheduled session");
    assert_eq!(
        reloaded
            .parts()
            .iter()
            .filter(|part| part.kind == "system_notification")
            .count(),
        1
    );
}

#[tokio::test]
async fn a_settle_whose_steer_reaches_a_turn_that_never_drains_it_still_wakes_the_model() {
    // Regression for the monitor-mode completion that went silent (QA session
    // 50, test F): the completion settle landed while the launching turn was
    // already in its final stop path — the execution was still registered, so
    // the steer send succeeded, but the loop's final `drain_steer_input` had
    // already passed and never polled again. The notification part existed;
    // the model was never woken. The settle must detect the unobserved steer
    // and start a fresh wake execution.
    let provider = Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["acknowledged the finished background operation".to_owned()],
        thinking_deltas: Vec::new(),
        finish_reason: Some(CompletionFinishReason::Stop),
    });
    let manager = manager_with_provider(provider).await;
    let session = create_with_model(&manager, "steer-drop regression", "fake", "fake-model").await;
    let session_id = session.id;

    // A completed launch receipt backed by a Running shell aggregate.
    let run_id = manager
        .store
        .start_run(
            session_id,
            "continue",
            run_marker_content("continue", Some("fake"), Some("fake-model"), None, None),
        )
        .await
        .expect("start launching run marker");
    install_test_background_operation(
        &manager,
        session_id,
        run_id,
        agena_storage::store::BackgroundOperationKind::Shell,
        "proc_dropped",
    )
    .await;

    // An "active" execution that never drains its steer and exits only when
    // the test signals it — the exact post-final-drain window of the bug.
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
    let owner = manager.background_handle();
    let stuck_execution = tokio::spawn(async move {
        owner
            .execute_registered(
                session_id,
                agena_domain::ExecutionSource::User,
                ExecutionConversationTarget::NewTurn,
                "steer-drop fixture",
                move |_manager, _control, steer_rx| async move {
                    // Never poll the steer: the loop's final drain already
                    // passed. Dropping the receiver on exit is what closes the
                    // steer channel.
                    let _steer_rx = steer_rx;
                    let _ = exit_rx.await;
                    Ok::<(), crate::AppError>(())
                },
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !manager.execution_registry.is_active(session_id).await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stuck execution registers");

    // The settle lands mid-"turn": it appends the notification and steers the
    // active execution, then must verify the steer landed.
    let notification = SystemNotificationContent {
        operation_id: "proc_dropped".to_string(),
        operation_kind: "shell".to_string(),
        status: "completed".to_string(),
        summary: "exit 0".to_string(),
        body: "exit 0".to_string(),
        ..Default::default()
    };
    let settle = tokio::spawn({
        let manager = manager.background_handle();
        async move {
            manager
                .settle_background_operation(
                    session_id,
                    "shell",
                    "proc_dropped",
                    PartState::Completed,
                    Ok("exit 0".to_owned()),
                    notification.clone(),
                )
                .await
        }
    });
    // Let the settle commit the notification part and send its steer.
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let session = manager
                .get_session(session_id)
                .await
                .expect("reload session");
            if session
                .parts()
                .iter()
                .any(|part| part.kind == "system_notification")
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("settle appends the notification part");

    // The "turn" exits without ever draining the steer.
    let _ = exit_tx.send(());
    let stuck_result = tokio::time::timeout(std::time::Duration::from_secs(2), stuck_execution)
        .await
        .expect("stuck execution exits")
        .expect("stuck execution joins");
    assert!(stuck_result.is_ok(), "stuck execution ends cleanly");

    // The settle must detect the unobserved steer and start a fresh wake
    // execution over the appended notification.
    let settled = tokio::time::timeout(std::time::Duration::from_secs(5), settle)
        .await
        .expect("settle completes")
        .expect("settle task joins");
    assert!(
        settled.is_ok(),
        "settle reports success: {:?}",
        settled.err()
    );

    let state = manager
        .session_store()
        .session_state(session_id)
        .await
        .expect("derive session state");
    assert_eq!(
        state.state,
        SessionState::Ready,
        "the session is woken and Ready, not left silent after the dropped steer"
    );
    let reloaded = manager
        .get_session(session_id)
        .await
        .expect("reload session");
    let runs = parts_into_runs(reloaded.parts());
    let wake_text = runs
        .iter()
        .rev()
        .find_map(|run| {
            let visible = run_visible_text_lossy(run);
            (!visible.trim().is_empty()).then_some(visible)
        })
        .expect("the wake turn produced a reply");
    assert_eq!(
        wake_text.trim(),
        "acknowledged the finished background operation",
        "the fresh wake execution ran a model turn over the notification"
    );
}
