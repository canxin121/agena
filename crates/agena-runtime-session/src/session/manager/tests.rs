//! Manager-level regression tests expressed exclusively through the v2
//! facade/parts model. Storage invariants, concurrency, recovery, retry,
//! usage, and JSONL have their exhaustive engine/facade suites in
//! `agena-storage` and `agena-storage-sqlite`; these tests prove the execution
//! manager's adapter preserves that model at its boundary.

use std::{collections::HashMap, sync::Arc};

use agena_domain::{ExecutionStatus, Role};
use agena_plugin_host::{PluginHost, PluginHostBuildConfig, PluginsConfig, ToolPresentationConfig};
use agena_storage::store::{NewPart, PartRole, PartState, PersistenceEngine, SessionState};
use sea_orm::{Database, DatabaseConnection};

use super::{SessionManager, build_message, merge_system_prompts};
use crate::session::store::MessageCheckpoint;
use crate::{
    RuntimeSessionManagerConfig,
    authorization::ExecutionPrincipal,
    message::{InteractiveRequestPart, MessageMetadata, PartContent, RequestPart},
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
    let ids = manager
        .store
        .reserve_message_ids(contents.len())
        .await
        .expect("reserve v2 placeholder ids");
    let message = build_message(
        ids,
        role,
        ExecutionStatus::Completed,
        contents,
        MessageMetadata::default(),
    )
    .expect("build message");
    session.messages.push(message.clone());
    manager
        .persist_session_changes(
            session,
            vec![MessageCheckpoint::all(&message)],
            None,
            manager.execution_state(),
        )
        .await
        .expect("persist message through facade")
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
    assert!(reloaded.messages.is_empty());

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
    let message = aggregate.messages.last_mut().expect("assistant message");
    let message_id = message.id;
    let changed_part_id = message.parts[1].id;
    message.parts[1].set_content(PartContent::text("right updated"));
    manager
        .persist_session_changes(
            aggregate,
            vec![MessageCheckpoint::part(message_id, changed_part_id)],
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
    assert_eq!(
        manager
            .get_session(child_id)
            .await
            .expect("project fork transcript")
            .messages[0]
            .as_text_lossy(),
        "shared prompt"
    );
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

    assert_eq!(child.messages.len(), 1);
    assert_eq!(
        child.messages[0].as_text_lossy(),
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
            at_message_id: Some(session.messages[0].id),
            title: Some("explicit marker fork child".to_owned()),
            expected_version: None,
        })
        .await
        .expect("fork at an explicit message marker");
    assert_eq!(explicit.messages.len(), 1);
    assert_eq!(
        explicit.messages[0].as_text_lossy(),
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
    assert_eq!(imported.messages.len(), 1);
    assert_eq!(imported.messages[0].as_text_lossy(), "round trip");
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
