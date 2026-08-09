//! Manager-level regression tests expressed exclusively through the v2
//! facade/parts model. Storage invariants, concurrency, recovery, retry,
//! usage, and JSONL have their exhaustive engine/facade suites in
//! `agena-storage` and `agena-storage-sqlite`; these tests prove the execution
//! manager's adapter preserves that model at its boundary.

use std::{collections::HashMap, sync::Arc};

use agena_domain::{
    AssistantReplyId, ExecutionStatus, ModelId, ModelRef, ProviderId, Role, TurnId,
};
use agena_plugin_host::{PluginHost, PluginHostBuildConfig, PluginsConfig, ToolPresentationConfig};
use agena_provider::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    CompletionUsage,
};
use agena_storage::store::{
    NewPart, PartDelta, PartRole, PartState, PartVisibility, PersistenceEngine, SessionState,
    SubmitOutcome,
};
use sea_orm::{Database, DatabaseConnection};

use super::{SessionManager, SessionRunRequest, SessionRunTermination, merge_system_prompts};
use crate::provider::{ModelRuntime, ProviderError};
use crate::session::store::{
    ProcessorPartIdAllocator, messages_from_parts, new_part_from_content, part_content_to_value,
    part_role_from_role, run_marker_content,
};
use crate::{
    RuntimeSessionManagerConfig,
    authorization::ExecutionPrincipal,
    message::{InteractiveRequestPart, PartContent, RequestPart},
    permission::{PermissionPolicy, ToolPermissionPolicy},
    provider::ProviderRegistry,
    session::{ContextGovernor, Session, SessionProcessor},
    tool::ToolExecutor,
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
        ToolPresentationConfig::default(),
    );
    let processor = SessionProcessor::new(
        Arc::new(ProviderRegistry::new()),
        ContextGovernor::new(agena_domain::ContextPolicy::default()),
        plugins,
        workspace_root,
    );
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("open v2 test database");
    initialize(&database).await;
    let manager = SessionManager::new(
        database.clone(),
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

async fn append_message(
    manager: &SessionManager,
    mut session: Session,
    role: Role,
    contents: Vec<PartContent>,
) -> Session {
    let new_parts = contents
        .iter()
        .map(|content| {
            new_part_from_content("text", part_role_from_role(role), content, PartState::Completed)
        })
        .collect::<Result<Vec<_>, _>>()
        .expect("build message parts");
    // A user append is one `user_send` run (marker + parts); any other role
    // appends content parts under a `continue` assistant run marker.
    let outcome = if role == Role::User {
        manager
            .store
            .submit_user_message(session.id, new_parts, None)
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
        vec![PartContent::text("first"), PartContent::text("second")],
    )
    .await;
    let session = append_message(
        &manager,
        session,
        Role::Assistant,
        vec![PartContent::text("answer")],
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
        vec![PartContent::text("left"), PartContent::text("right")],
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
        changed.content = part_content_to_value(&PartContent::text("right updated"))
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
        vec![PartContent::text("shared prompt")],
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
    let child_messages = messages_from_parts(
        manager
            .get_session(child_id)
            .await
            .expect("project fork transcript")
            .parts(),
    )
    .expect("project fork transcript");
    assert_eq!(child_messages[0].as_text_lossy(), "shared prompt");
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
            PartContent::text("shared first"),
            PartContent::text("shared second"),
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

    let child_messages = messages_from_parts(child.parts()).expect("project child transcript");
    assert_eq!(child_messages.len(), 1);
    assert_eq!(
        child_messages[0].as_text_lossy(),
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
            at_message_id: Some(
                messages_from_parts(session.parts())
                    .expect("project source transcript")[0]
                    .id,
            ),
            title: Some("explicit marker fork child".to_owned()),
            expected_version: None,
        })
        .await
        .expect("fork at an explicit message marker");
    let explicit_messages =
        messages_from_parts(explicit.parts()).expect("project explicit fork transcript");
    assert_eq!(explicit_messages.len(), 1);
    assert_eq!(
        explicit_messages[0].as_text_lossy(),
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
                serde_json::to_value(PartContent::request(RequestPart::UserInput(
                    InteractiveRequestPart::pending(agena_domain::UserInputRequest {
                        request_id: "ask-1".to_owned(),
                        session_id: Some(session.id),
                        title: "Choose a path".to_owned(),
                        kind: "ask_user".to_owned(),
                        auto_resolution_ms: None,
                        presented_at: None,
                        questions: Vec::new(),
                        created_at: chrono::Utc::now(),
                    }),
                )))
                .expect("serialize interaction part"),
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
        vec![PartContent::text("round trip")],
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
    let imported_messages = messages_from_parts(imported.parts()).expect("project imported transcript");
    assert_eq!(imported_messages.len(), 1);
    assert_eq!(imported_messages[0].as_text_lossy(), "round trip");
}

#[tokio::test]
async fn query_projection_is_derived_from_persisted_parts() {
    let manager = test_manager().await;
    let session = create(&manager, "query projection").await;
    let session = append_message(
        &manager,
        session,
        Role::User,
        vec![PartContent::text("query me")],
    )
    .await;
    let projected = manager
        .list_projected_messages(session.id, true)
        .await
        .expect("list projected messages");
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].parts.len(), 1);
    assert_eq!(
        projected[0].parts[0]
            .content
            .as_ref()
            .and_then(PartContent::text_value),
        Some("query me")
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

#[async_trait::async_trait]
impl ModelRuntime for FakeProvider {
    fn id(&self) -> &str {
        self.provider_id
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    async fn list_models(
        &self,
    ) -> Result<Vec<agena_domain::Model>, ProviderError> {
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
                dyn futures_util::Stream<
                        Item = Result<CompletionStreamEvent, ProviderError>,
                    > + Send,
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
        ToolPresentationConfig::default(),
    );
    let mut registry = ProviderRegistry::new();
    registry.register_arc(provider);
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(agena_domain::ContextPolicy::default()),
        plugins,
        workspace_root,
    );
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("open v2 test database");
    initialize(&database).await;
    SessionManager::new(
        database,
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
        turn_id,
        reply_id,
        session_id: session.id,
        model_turn_id: None,
        completion_parent_message_id: None,
        model: ModelRef::new("fake", "fake-model"),
        model_thinking_mode: None,
        model_speed_mode: None,
        completion: CompletionRequest {
            model: ModelId::new("fake-model"),
            system: None,
            messages: Vec::new(),
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
        part_ids: ProcessorPartIdAllocator,
        next_call_id: 0,
        store: manager.store.clone(),
        cancel: None,
    };

    let result = manager
        .execution_state()
        .processor
        .run_turn(run)
        .await
        .expect("stream one assistant turn through the parts-native processor");

    // Terminal Completed: the marker and every content part end terminal.
    assert!(matches!(result.termination, SessionRunTermination::Completed));
    assert_eq!(result.message_state, ExecutionStatus::Completed);
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
    assert_eq!(think_parts.len(), 1, "thinking delta becomes one think part");
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
        .start_run(session.id, "continue", serde_json::json!({"run_kind": "continue"}))
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
