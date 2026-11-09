use super::super::tests::fixture;
use super::*;

fn create(key: &str, name: &str) -> BotCreate {
    BotCreate {
        request_key: key.into(),
        name: name.into(),
        persona: None,
    }
}
fn archive(key: &str, p: &BotProfile) -> BotArchive {
    BotArchive {
        request_key: key.into(),
        expected_revision: p.revision,
    }
}
fn restart(manager: &SessionManager) -> SessionManager {
    let state = manager.state.load_full();
    let restarted = SessionManager::new(
        manager.kernel_runtime.clone(),
        state.providers.clone(),
        state.governor.clone(),
        state.processor.clone(),
        state.executor.clone(),
        state.config.clone(),
    );
    if let Some(repository) = manager.memory_preferences.get() {
        restarted.install_memory_preferences(repository.clone());
    }
    restarted
}

#[tokio::test]
async fn strict_registry_rejects_legacy_fields_and_ignores_legacy_keys() {
    let (manager, ordinary) = fixture().await;
    manager
        .save_bot_state("operator_bot_v1", &ordinary.get())
        .await
        .unwrap();
    manager
        .save_bot_state("operator_bot_seed_v1", &123_i64)
        .await
        .unwrap();
    let profile = manager.ensure_default_bot_profile().await.unwrap();
    assert_ne!(profile.session_id, ordinary.get());
    assert_eq!(
        manager
            .kernel()
            .reader
            .metadata(session_id(profile.session_id).unwrap())
            .await
            .unwrap()
            .workspace_id,
        None
    );
    let current = manager.bot_state::<Value>(REGISTRY).await.unwrap().unwrap();
    assert_eq!(current.as_object().unwrap().len(), 4);
    for field in ["workspaces", "global_scope"] {
        let mut old = current.clone();
        old[field] = serde_json::json!({});
        manager.save_bot_state(REGISTRY, &old).await.unwrap();
        let error = manager
            .ensure_default_bot_profile()
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("one-time migration"), "{error}");
    }
    let mut old = current;
    let profile = old["bots"][0].clone();
    old["pending"] = serde_json::json!({"profile": profile, "command": RequestId::new(), "title": "Bot", "receipt": null, "workspace": 1});
    manager.save_bot_state(REGISTRY, &old).await.unwrap();
    let error = manager
        .ensure_default_bot_profile()
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("one-time migration"), "{error}");
}

#[tokio::test]
async fn concurrent_default_creation_is_global_and_idempotent() {
    let (manager, _) = fixture().await;
    let other = restart(&manager);
    let (a, b) = tokio::join!(
        manager.ensure_default_bot_profile(),
        other.ensure_default_bot_profile()
    );
    let a = a.unwrap();
    assert_eq!(a, b.unwrap());
    assert_eq!(
        manager
            .kernel()
            .reader
            .metadata(session_id(a.session_id).unwrap())
            .await
            .unwrap()
            .workspace_id,
        None
    );
    assert_eq!(
        manager.list_bot_profiles(false).await.unwrap().items.len(),
        1
    );
}

#[tokio::test]
async fn default_creation_without_any_projects() {
    let (template, _) = fixture().await;
    let state = template.state.load_full();
    let kernel = Arc::new(
        agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap(),
    );
    let manager = SessionManager::new(
        kernel.clone(),
        state.providers.clone(),
        state.governor.clone(),
        state.processor.clone(),
        state.executor.clone(),
        state.config.clone(),
    );
    let query = agena_storage::WorkspaceListQuery {
        limit: 100,
        ..Default::default()
    };
    assert!(
        kernel
            .workspaces
            .list(query.clone())
            .await
            .unwrap()
            .is_empty()
    );
    let profile = manager.ensure_default_bot_profile().await.unwrap();
    assert_eq!(
        kernel
            .ports
            .reader
            .metadata(session_id(profile.session_id).unwrap())
            .await
            .unwrap()
            .workspace_id,
        None
    );
    assert!(kernel.workspaces.list(query).await.unwrap().is_empty());
}

#[tokio::test]
async fn names_revision_and_lost_ack_updates_are_isolated() {
    let (manager, _) = fixture().await;
    let first = manager.ensure_default_bot_profile().await.unwrap();
    let second = manager
        .create_bot_profile(create("second", "  Straße  "))
        .await
        .unwrap();
    assert_eq!(second.name, "Straße");
    assert_eq!(
        manager
            .kernel()
            .reader
            .metadata(session_id(second.session_id).unwrap())
            .await
            .unwrap()
            .workspace_id,
        None
    );
    let context = manager.bot_context(second.session_id).await.unwrap();
    assert!(context.get("workspace").is_none());
    assert!(context.get("workspace_id").is_none());
    assert!(
        manager
            .create_bot_profile(create("duplicate", "STRASSE"))
            .await
            .is_err()
    );
    for name in ["", "  ", "bad\n", "\u{0}bot"] {
        assert!(
            manager
                .create_bot_profile(create(name, name))
                .await
                .is_err()
        );
    }
    let update = BotUpdate {
        request_key: "update".into(),
        expected_revision: second.revision,
        name: "Helper".into(),
        persona: BotPersona {
            instructions: "Only my personality".into(),
        },
    };
    let changed = manager
        .update_bot_profile(second.id, update.clone())
        .await
        .unwrap();
    let restarted = restart(&manager);
    assert_eq!(
        restarted
            .update_bot_profile(second.id, update.clone())
            .await
            .unwrap(),
        changed
    );
    assert_eq!(restarted.get_bot_profile(first.id).await.unwrap(), first);
    let mut stale = update.clone();
    stale.request_key = "new-stale".into();
    assert!(matches!(
        restarted.update_bot_profile(second.id, stale).await,
        Err(AppError::BotConflict(_))
    ));
    let mut changed_retry = update;
    changed_retry.name = "Changed retry".into();
    assert!(
        restarted
            .update_bot_profile(second.id, changed_retry)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn concurrent_managers_create_once_and_recover_committed_native_pending_receipt() {
    let (manager, _) = fixture().await;
    manager.ensure_default_bot_profile().await.unwrap();
    let other = restart(&manager);
    let request = create("concurrent", "Concurrent");
    let (a, b) = tokio::join!(
        manager.create_bot_profile(request.clone()),
        other.create_bot_profile(request.clone())
    );
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(
        manager.list_bot_profiles(false).await.unwrap().items.len(),
        2
    );
    // Inject exactly the durable boundary after native create and before the
    // registry's completed receipt. Replay must retain title/selection and UUID.
    let mut registry = manager.load_registry().await.unwrap();
    let profile = BotProfile {
        id: Uuid::new_v4(),
        name: "Crash recovery".into(),
        persona: Default::default(),
        session_id: 0,
        revision: 1,
        archived: false,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    let command = RequestId::new();
    let request = create("crash", "Crash recovery");
    registry.pending = Some(Creation {
        title: profile.name.clone(),
        profile: profile.clone(),
        command,
        receipt: Some((
            "create:crash".into(),
            serde_json::to_value(&request).unwrap(),
        )),
    });
    manager.save_registry(&registry).await.unwrap();
    let committed = manager
        .kernel()
        .sessions
        .create_global(command, profile.name.clone(), Default::default())
        .await
        .unwrap()
        .value
        .id;
    manager
        .rename_session(committed.get(), "Preserve committed native title".into())
        .await
        .unwrap();
    let restarted = restart(&manager);
    let recovered = restarted.create_bot_profile(request).await.unwrap();
    assert_eq!(recovered.id, profile.id);
    assert_eq!(recovered.session_id, committed.get());
    assert_eq!(
        restarted
            .kernel()
            .reader
            .metadata(committed)
            .await
            .unwrap()
            .title,
        "Preserve committed native title"
    );
    assert_eq!(
        restarted
            .list_bot_profiles(false)
            .await
            .unwrap()
            .items
            .len(),
        3
    );
}

#[tokio::test]
async fn deleted_sessions_recover_independently_and_all_roots_stay_excluded() {
    let (manager, ordinary) = fixture().await;
    let first = manager.ensure_default_bot_profile().await.unwrap();
    let second = manager
        .create_bot_profile(create("second", "Second"))
        .await
        .unwrap();
    for p in [&first, &second] {
        manager
            .kernel()
            .sessions
            .delete(RequestId::new(), session_id(p.session_id).unwrap())
            .await
            .unwrap();
    }
    let (a, b) = tokio::join!(
        manager.ensure_named_bot_session(first.id),
        manager.ensure_named_bot_session(second.id)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.bot_id, first.id);
    assert_eq!(b.bot_id, second.id);
    assert_ne!(a.session_id, b.session_id);
    assert_ne!(a.session_id, first.session_id);
    assert_ne!(b.session_id, second.session_id);
    assert!(
        manager
            .bot_target(a.session_id, Some(b.session_id))
            .await
            .is_err()
    );
    assert!(
        manager
            .bot_target(b.session_id, Some(a.session_id))
            .await
            .is_err()
    );
    assert_eq!(
        manager
            .bot_target(a.session_id, Some(ordinary.get()))
            .await
            .unwrap(),
        ordinary.get()
    );
    for id in [
        first.session_id,
        second.session_id,
        a.session_id,
        b.session_id,
    ] {
        assert!(manager.is_reserved_bot_session(id).await.unwrap());
    }
    assert!(
        manager
            .bot_profile_for_session(first.session_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn archive_revokes_authority_preserves_history_and_never_resurrects_last_bot() {
    let (manager, _) = fixture().await;
    let first = manager.ensure_default_bot_profile().await.unwrap();
    let second = manager
        .create_bot_profile(create("second", "Second"))
        .await
        .unwrap();
    let request = archive("archive-first", &first);
    let archived = manager
        .archive_bot_profile(first.id, request.clone())
        .await
        .unwrap();
    assert!(archived.archived);
    assert!(
        manager
            .require_writable_bot_history(first.session_id)
            .await
            .is_err()
    );
    assert!(matches!(
        manager
            .deliver_companion_notice(
                first.session_id,
                1,
                "archived-reminder",
                "never wake archived Bot"
            )
            .await,
        Err(AppError::BotConflict(_))
    ));
    assert!(matches!(
        manager
            .deliver_scheduled_job(
                first.session_id,
                "job".into(),
                "delivery".into(),
                "never wake archived Bot".into(),
                None
            )
            .await,
        Err(AppError::BotConflict(_))
    ));
    assert!(
        manager
            .kernel()
            .reader
            .metadata(session_id(first.session_id).unwrap())
            .await
            .is_ok()
    );
    assert_eq!(
        manager.ensure_default_bot_profile().await.unwrap().id,
        second.id
    );
    assert_eq!(
        manager
            .archive_bot_profile(first.id, request)
            .await
            .unwrap(),
        archived
    );
    assert_eq!(
        manager.bot_context(first.session_id).await.unwrap()["enabled"],
        false
    );
    assert!(
        manager
            .require_operator_bot(first.session_id)
            .await
            .is_err()
    );
    assert!(
        manager
            .bot_target(second.session_id, Some(first.session_id))
            .await
            .is_err()
    );
    assert!(manager.ensure_named_bot_session(first.id).await.is_err());
    manager
        .archive_bot_profile(second.id, archive("archive-second", &second))
        .await
        .unwrap();
    let restarted = restart(&manager);
    assert!(matches!(
        restarted.ensure_bot_session().await,
        Err(AppError::BotNotFound(_))
    ));
    assert_eq!(
        restarted.list_bot_profiles(false).await.unwrap(),
        BotList {
            items: vec![],
            default_bot_id: None
        }
    );
    assert_eq!(
        restarted.list_bot_profiles(true).await.unwrap().items.len(),
        2
    );
    assert!(
        restarted
            .create_bot_profile(create("reuse-name", &first.name))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn idle_archive_checks_native_queue_and_pending_cards_without_runtime_activity() {
    let (manager, _) = fixture().await;
    let profile = manager.ensure_default_bot_profile().await.unwrap();
    let session = session_id(profile.session_id).unwrap();
    let execution = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "pending native work".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    // No supervisor/RunControl has been installed: durable work is sufficient.
    assert!(manager.active.lock().unwrap().is_empty());
    assert!(matches!(
        manager
            .archive_bot_profile(profile.id, archive("queued", &profile))
            .await,
        Err(AppError::BotConflict(_))
    ));
    let attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "pending".into(),
                name: "ask".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.request_interaction(
        RequestId::new(),
        InteractionRequest::Permission {
            action_key: "test".into(),
            description: "native pending permission".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        !manager
            .kernel()
            .reader
            .control_snapshot(session, 1)
            .await
            .unwrap()
            .pending_interactions
            .interactions
            .is_empty()
    );
    assert!(matches!(
        manager
            .archive_bot_profile(profile.id, archive("card", &profile))
            .await,
        Err(AppError::BotConflict(_))
    ));
    assert!(!manager.get_bot_profile(profile.id).await.unwrap().archived);
}

#[tokio::test]
async fn discovery_excludes_every_bot_root_and_descendant_even_after_archive_and_recovery() {
    let (manager, ordinary) = fixture().await;
    let first = manager.ensure_default_bot_profile().await.unwrap();
    let second = manager
        .create_bot_profile(create("second", "Second"))
        .await
        .unwrap();
    let mut excluded = vec![first.session_id, second.session_id];
    for p in [&first, &second] {
        for relation in [
            agena_domain::SessionRelationKind::Child,
            agena_domain::SessionRelationKind::Fork,
        ] {
            let id = manager
                .kernel()
                .sessions
                .create_global(
                    RequestId::new(),
                    "private Bot continuation".into(),
                    SessionConfiguration {
                        parent_session_id: Some(session_id(p.session_id).unwrap()),
                        relation_kind: relation,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .value
                .id
                .get();
            excluded.push(id);
        }
    }
    manager
        .archive_bot_profile(second.id, archive("archive", &second))
        .await
        .unwrap();
    manager
        .kernel()
        .sessions
        .delete(RequestId::new(), session_id(first.session_id).unwrap())
        .await
        .unwrap();
    let replacement = manager.ensure_named_bot_session(first.id).await.unwrap();
    excluded.push(replacement.session_id);
    let r: BotRequest = serde_json::from_value(json!({"action":"sessions","limit":50})).unwrap();
    let result = manager
        .bot_discovery(replacement.session_id, &r)
        .await
        .unwrap();
    let ids: Vec<i64> = result["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["session_id"].as_i64().unwrap())
        .collect();
    assert!(ids.contains(&ordinary.get()));
    for id in excluded {
        assert!(!ids.contains(&id), "leaked Bot root or descendant {id}");
        assert!(
            manager
                .bot_target(replacement.session_id, Some(id))
                .await
                .is_err()
        );
    }
    assert!(manager.bot_discovery(second.session_id, &r).await.is_err());
}

#[tokio::test]
async fn concurrent_profile_writes_have_one_winner_and_retries_keep_the_original_receipt() {
    let (manager, _) = fixture().await;
    let profile = manager.ensure_default_bot_profile().await.unwrap();
    let other = restart(&manager);
    let a = BotUpdate {
        request_key: "a".into(),
        expected_revision: profile.revision,
        name: "A".into(),
        persona: Default::default(),
    };
    let b = BotUpdate {
        request_key: "b".into(),
        expected_revision: profile.revision,
        name: "B".into(),
        persona: Default::default(),
    };
    let (left, right) = tokio::join!(
        manager.update_bot_profile(profile.id, a.clone()),
        other.update_bot_profile(profile.id, b.clone())
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let (winner, request) = if let Ok(winner) = left {
        (winner, a)
    } else {
        (right.unwrap(), b)
    };
    manager
        .archive_bot_profile(winner.id, archive("archive", &winner))
        .await
        .unwrap();
    assert_eq!(
        other.update_bot_profile(profile.id, request).await.unwrap(),
        winner
    );
    assert!(manager.get_bot_profile(profile.id).await.unwrap().archived);
}

#[tokio::test]
async fn default_needs_no_project_preferences_and_many_bots_remain_independent() {
    let (manager, _) = fixture().await;
    let state = manager.state.load_full();
    let uninstalled = SessionManager::new(
        manager.kernel_runtime.clone(),
        state.providers.clone(),
        state.governor.clone(),
        state.processor.clone(),
        state.executor.clone(),
        state.config.clone(),
    );
    let first = uninstalled.ensure_default_bot_profile().await.unwrap();
    assert_eq!(manager.ensure_default_bot_profile().await.unwrap(), first);
    let mut ids = std::collections::HashSet::from([first.session_id]);
    for i in 0..12 {
        let p = manager
            .create_bot_profile(create(&format!("create-{i}"), &format!("Bot {i}")))
            .await
            .unwrap();
        assert!(ids.insert(p.session_id));
        assert_eq!(p.persona, BotPersona::default());
        assert_eq!(
            manager
                .ensure_named_bot_session(p.id)
                .await
                .unwrap()
                .session_id,
            p.session_id
        );
    }
    assert_eq!(
        manager.list_bot_profiles(false).await.unwrap().items.len(),
        13
    );
    assert_eq!(manager.ensure_default_bot_profile().await.unwrap(), first);
}

#[tokio::test]
async fn observational_and_unknown_uuid_lookups_do_not_create_hidden_bots() {
    let (manager, ordinary) = fixture().await;
    let count = manager
        .kernel()
        .reader
        .query_summaries(SummaryQuery {
            limit: 128,
            ..Default::default()
        })
        .await
        .unwrap()
        .sessions
        .len();
    assert!(
        manager
            .bot_profile_for_session(ordinary.get())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !manager
            .is_reserved_bot_session(ordinary.get())
            .await
            .unwrap()
    );
    assert_eq!(
        manager.bot_context(ordinary.get()).await.unwrap(),
        json!({"enabled":false})
    );
    let unknown = Uuid::new_v4();
    assert!(matches!(
        manager.get_bot_profile(unknown).await,
        Err(AppError::BotNotFound(_))
    ));
    assert!(matches!(
        manager.ensure_named_bot_session(unknown).await,
        Err(AppError::BotNotFound(_))
    ));
    assert!(
        manager
            .bot_state::<Value>(REGISTRY)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .query_summaries(SummaryQuery {
                limit: 128,
                ..Default::default()
            })
            .await
            .unwrap()
            .sessions
            .len(),
        count
    );
}

#[tokio::test]
async fn bounded_update_receipts_keep_create_archive_and_expired_retries_are_stale_noops() {
    let (manager, _) = fixture().await;
    manager.ensure_default_bot_profile().await.unwrap();
    let request = create("retained-create", "Retained");
    let initial = manager.create_bot_profile(request.clone()).await.unwrap();
    let first = BotUpdate {
        request_key: "expired".into(),
        expected_revision: initial.revision,
        name: initial.name.clone(),
        persona: Default::default(),
    };
    let mut current = manager
        .update_bot_profile(initial.id, first.clone())
        .await
        .unwrap();
    for i in 0..UPDATE_RECEIPTS + 1 {
        current = manager
            .update_bot_profile(
                initial.id,
                BotUpdate {
                    request_key: format!("update-{i}"),
                    expected_revision: current.revision,
                    name: initial.name.clone(),
                    persona: Default::default(),
                },
            )
            .await
            .unwrap();
    }
    let registry = manager.load_registry().await.unwrap();
    assert_eq!(
        registry
            .receipts
            .keys()
            .filter(|key| key.starts_with("update:"))
            .count(),
        UPDATE_RECEIPTS
    );
    assert!(
        !registry
            .receipts
            .contains_key(&format!("update:{}:expired", initial.id))
    );
    assert!(matches!(
        manager.update_bot_profile(initial.id, first).await,
        Err(AppError::BotConflict(_))
    ));
    assert_eq!(manager.get_bot_profile(initial.id).await.unwrap(), current);
    assert_eq!(manager.create_bot_profile(request).await.unwrap(), initial);
    let archive_request = archive("retained-archive", &current);
    let archived = manager
        .archive_bot_profile(initial.id, archive_request.clone())
        .await
        .unwrap();
    assert_eq!(
        manager
            .archive_bot_profile(initial.id, archive_request)
            .await
            .unwrap(),
        archived
    );
}

#[tokio::test]
async fn archive_refuses_active_native_subagent_of_an_idle_parent() {
    let (manager, _) = fixture().await;
    let p = manager.ensure_default_bot_profile().await.unwrap();
    manager
        .kernel()
        .sessions
        .create_global(
            RequestId::new(),
            "Native Subagent".into(),
            SessionConfiguration {
                parent_session_id: Some(session_id(p.session_id).unwrap()),
                relation_kind: agena_domain::SessionRelationKind::Subagent,
                agent: Some(SessionAgentConfiguration {
                    name: "archive_probe".into(),
                    status: agena_domain::AgentStatus::Created,
                    started_at_ms: None,
                    finished_at_ms: None,
                    failure: None,
                    limits: Default::default(),
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let snapshot = manager
        .kernel()
        .reader
        .control_snapshot(session_id(p.session_id).unwrap(), 1)
        .await
        .unwrap();
    assert_eq!(snapshot.summary.running_executions, 0);
    assert_eq!(snapshot.summary.queued_executions, 0);
    assert!(matches!(
        manager
            .archive_bot_profile(p.id, archive("active-child", &p))
            .await,
        Err(AppError::BotConflict(_))
    ));
}

#[tokio::test]
async fn archive_refuses_native_background_activity_after_the_source_execution_finishes() {
    use agena_storage::kernel::{
        BackgroundActivityKind, BackgroundActivityOutcome, BackgroundActivityStart,
    };
    let (manager, _) = fixture().await;
    let p = manager.ensure_default_bot_profile().await.unwrap();
    let session = session_id(p.session_id).unwrap();
    let execution = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "native background launch".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "launch".into(),
                name: "background".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let activity = tool
        .begin_activity(
            RequestId::new(),
            BackgroundActivityStart {
                kind: BackgroundActivityKind::Process,
                label: "independent lease".into(),
            },
        )
        .await
        .unwrap()
        .value;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"started":true}),
        },
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let snapshot = manager
        .kernel()
        .reader
        .control_snapshot(session, 1)
        .await
        .unwrap();
    assert_eq!(snapshot.summary.running_executions, 0);
    assert_eq!(snapshot.summary.queued_executions, 0);
    assert!(matches!(
        manager
            .archive_bot_profile(p.id, archive("background", &p))
            .await,
        Err(AppError::BotConflict(_))
    ));
    activity
        .end(RequestId::new(), BackgroundActivityOutcome::Stopped)
        .await
        .unwrap();
    assert!(
        manager
            .archive_bot_profile(p.id, archive("stopped", &p))
            .await
            .unwrap()
            .archived
    );
}

#[tokio::test]
async fn long_persona_receipts_are_evicted_by_byte_budget_not_only_by_count() {
    let (manager, _) = fixture().await;
    manager.ensure_default_bot_profile().await.unwrap();
    let initial = manager
        .create_bot_profile(create("long-persona-byte-budget", "Long"))
        .await
        .unwrap();
    // Each receipt stores the full request payload and result, so a handful of
    // ~64 KiB character sheets exceeds the receipt byte budget well before the
    // 256-entry count cap.
    let persona = "你是一只鲸鱼娘。".repeat(8 * 1024);
    let mut current = initial.clone();
    for i in 0..40 {
        current = manager
            .update_bot_profile(
                initial.id,
                BotUpdate {
                    request_key: format!("long-update-{i}"),
                    expected_revision: current.revision,
                    name: initial.name.clone(),
                    persona: BotPersona {
                        instructions: format!("{persona}{i}"),
                    },
                },
            )
            .await
            .unwrap();
    }
    let registry = manager.load_registry().await.unwrap();
    let bytes: usize = registry
        .receipts
        .iter()
        .map(|(key, receipt)| key.len() + receipt_size(receipt))
        .sum();
    assert!(
        bytes <= MAX_RECEIPT_BYTES,
        "registry receipts grew to {bytes} bytes"
    );
    assert!(
        registry
            .receipts
            .keys()
            .any(|key| key == &format!("update:{}:long-update-39", initial.id)),
        "the newest receipt must survive eviction"
    );
    // The live profile, not the receipt history, is the durable truth.
    assert_eq!(manager.get_bot_profile(initial.id).await.unwrap(), current);
}
