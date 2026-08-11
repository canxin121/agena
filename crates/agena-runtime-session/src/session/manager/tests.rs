//! Manager-level regression tests expressed exclusively through the v2
//! facade/parts model. Storage invariants, concurrency, recovery, retry,
//! usage, and JSONL have their exhaustive engine/facade suites in
//! `agena-storage` and `agena-storage-sqlite`; these tests prove the execution
//! manager's adapter preserves that model at its boundary.

use std::{collections::HashMap, sync::Arc};

use agena_domain::{
    AssistantReasoningField, AssistantReplyId, ModelId, ModelRef, ProviderId, Role,
    StructuredObject, TimeRange, ToolInvocation, TurnId,
};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_provider::{
    CompletionFinishReason, CompletionInputPart, CompletionRequest, CompletionResponse,
    CompletionStreamEvent, CompletionUsage,
};
use agena_storage::store::{
    NewPart, PartDelta, PartRole, PartState, PartVisibility, PersistenceEngine, SessionState,
    SubmitOutcome,
};
use sea_orm::{Database, DatabaseConnection};

use super::{
    ExecutionConversationTarget, SessionManager, SessionRunRequest, SessionRunTermination,
    SessionUserRunRequest, merge_system_prompts,
};
use crate::provider::{ModelRuntime, ProviderError};
use crate::session::manager::runs::run_visible_text_lossy;
use crate::session::store::{
    ProcessorPartIdAllocator, interaction_from_request, new_part_from_content, parts_into_runs,
    run_marker_content, text_content, tool_call_from_operation, typed_content_to_value,
};
use crate::{
    ContextGovernor, RuntimeSessionManagerConfig,
    authorization::ExecutionPrincipal,
    part::{InteractiveRequestPart, RequestPart},
    permission::{PermissionPolicy, ToolPermissionPolicy},
    provider::ProviderRegistry,
    session::{Session, SessionProcessor},
    tool::ToolExecutor,
};
use agena_runtime_contracts::part_content::TypedContent;

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
    let processor = SessionProcessor::new(plugins, workspace_root);
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
        .cancel_active_execution(session_id)
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
    manager
        .store
        .append_parts(
            session.id,
            run_id,
            vec![NewPart::pending(
                "interaction",
                PartRole::Runtime,
                interaction_from_request(&RequestPart::UserInput(InteractiveRequestPart::pending(
                    agena_domain::UserInputRequest {
                        request_id: "ask-1".to_owned(),
                        session_id: Some(session.id),
                        title: "Choose a path".to_owned(),
                        kind: "ask_user".to_owned().into(),
                        auto_resolution_ms: None,
                        presented_at: None,
                        questions: Vec::new(),
                        created_at: chrono::Utc::now(),
                    },
                )))
                .as_value(),
            )],
        )
        .await
        .expect("append pending interaction");

    // Fault injection through the persistence engine: an open session must
    // derive AwaitingUser even when the paused run has no lease. Production
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
    assert_eq!(before.state, SessionState::AwaitingUser);

    // Exercise the manager's lazy reconciliation-on-open path. If its
    // AwaitingUser guard regresses, this call terminalizes the paused run.
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
    let interaction = after
        .parts
        .iter()
        .find(|part| part.kind == "interaction")
        .expect("interaction remains");
    assert!(marker.state.is_in_flight(), "paused run is not aborted");
    assert_eq!(interaction.state, PartState::Pending);
    assert_eq!(
        facade
            .session_state(session.id)
            .await
            .expect("derive state after open")
            .state,
        SessionState::AwaitingUser
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
        .list_projected_runs(session.id, true)
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
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::Completed,
                },
                NewPart {
                    kind: "tool_call".to_owned(),
                    role: PartRole::Assistant,
                    content: serde_json::json!({"name": "tools_search", "input": {"query": "x"}}),
                    summary: None,
                    visibility: PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::Completed,
                },
            ],
        )
        .await
        .expect("append assistant parts");

    let projected = manager
        .list_projected_runs(session.id, true)
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

struct StartupFailureProvider {
    model: ModelId,
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
    let processor = SessionProcessor::new(plugins, workspace_root);
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
    let processor = SessionProcessor::new(plugins, workspace_root);
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
        marker_content: Some(run_marker_content("continue", None, None, None, None)),
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
                rendered_markdown: None,
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
        .expect("tools_search part");
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
    let operation = agena_runtime_contracts::part_content::operation_from_tool_call(&tool_call);
    assert!(
        operation.output_text().is_some_and(|output| {
            output.contains("Matching tools for \"filesystem\"") && output.contains("fs.read")
        }),
        "tools_search must execute through the Tool API handler: {operation:?}"
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
    assert!(completed.parts().iter().any(|part| {
        part.kind == "text"
            && part.content.get("text").and_then(serde_json::Value::as_str)
                == Some("MIXED_TOOL_BATCH_OK")
    }));
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
        "Rename session",
        TimeRange {
            start_ms: 1,
            end_ms: None,
        },
    );
    let tool_part = new_part_from_content(
        "tool_call",
        PartRole::Assistant,
        &TypedContent::ToolCall(tool_call_from_operation(&operation)),
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
    // applies before suspending on the reply; the interaction part is created
    // and the tool part must survive the suspension without an invalid
    // `in_progress -> pending` downgrade.
    let request = crate::part::AskUserToolInput {
        title: "Approve New Plan".to_owned(),
        kind: "review".to_owned(),
        auto_resolution_ms: None,
        questions: Vec::new(),
    };
    manager
        .apply_user_input_request_with_id(
            session.clone(),
            &pending_tool,
            request,
            "host-input:1:1:0".to_owned(),
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
        persisted
            .parts
            .iter()
            .any(|part| part.kind == "interaction" && part.state.is_in_flight()),
        "the host ask_user request is recorded as a pending interaction part"
    );
}

/// The canonical `interaction` part created by a host ask_user (plan approval)
/// must be resolvable by the reply machinery: it carries the flat
/// `request_id`/`tool_part_id`/`operation_id` correlation keys alongside the
/// canonical `request` object, so replying to it does not error with "pending
/// user input request not found" (regression: plan approval dialog selection
/// failed because the reply lookup only read the flat v1 keys).
#[tokio::test]
async fn host_ask_user_interaction_part_is_reply_resolvable() {
    let (manager, _database) = test_manager_with_database().await;
    let session = create(&manager, "host ask_user reply resolution").await;

    let operation = agena_runtime_contracts::part::OperationPart::pending(
        1,
        ToolInvocation::new("plan.set", StructuredObject::default()),
        "Create plan",
        TimeRange { start_ms: 1, end_ms: None },
    );
    let tool_part = new_part_from_content(
        "tool_call",
        PartRole::Assistant,
        &TypedContent::ToolCall(tool_call_from_operation(&operation)),
        PartState::InProgress,
    )
    .expect("build tool part");
    let store = manager.session_store();
    let outcome = store
        .submit_user_run(session.id, manager.store.owner_id.as_str(), vec![tool_part], None)
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
        auto_resolution_ms: None,
        questions: Vec::new(),
    };
    manager
        .apply_user_input_request_with_id(
            session.clone(),
            &pending_tool,
            request,
            "host-input:1:1:0".to_owned(),
            manager.execution_state(),
        )
        .await
        .expect("request host user input");

    let persisted = store.load(session.id).await.expect("reload parts");
    let interaction = persisted
        .parts
        .iter()
        .find(|part| part.kind == "interaction")
        .expect("interaction part exists");
    let interaction_part_id = interaction.part_id;
    // The canonical writer must also emit the flat keys the reply machinery
    // correlates on, otherwise the TUI's reply dispatch reports "pending user
    // input request not found" and no dialog option can be selected.
    assert_eq!(
        interaction.content.get("request_id").and_then(serde_json::Value::as_str),
        Some("host-input:1:1:0"),
        "flat request_id mirrors the canonical request id"
    );
    assert_eq!(
        interaction
            .content
            .get("tool_part_id")
            .and_then(serde_json::Value::as_i64),
        Some(tool_part_id),
        "tool_part_id correlates the interaction to its suspended tool part"
    );
    assert!(
        interaction.content.get("operation_id").is_some(),
        "operation_id is recorded for operation correlation"
    );

    let session = manager
        .store
        .load_session(session.id)
        .await
        .expect("reload submitted run as a session");
    let pending = super::replies::find_pending_user_input_by_request_id(
        &session,
        "host-input:1:1:0",
    )
    .expect("reply lookup resolves the canonical interaction part");
    assert_eq!(pending.tool.part.part_id, tool_part_id);
    assert_eq!(pending.request.part_id, interaction_part_id);
    let resolved_request = super::replies::pending_user_input_request(&session, &pending)
        .expect("request payload is recoverable");
    assert_eq!(resolved_request.title, "Approve New Plan");
    assert_eq!(resolved_request.kind, agena_domain::UserInputKind::Review);
}
