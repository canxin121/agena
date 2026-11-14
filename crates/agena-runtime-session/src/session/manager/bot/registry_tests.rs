//! Bot identity table and conversation-ownership tests.
//!
//! The table is an ordinary SQLite relation now, so these tests cover the
//! properties that made the old JSON registry correct: one winner under
//! concurrency, idempotent request keys, revision compare-and-set, ownership
//! recorded on conversations, and an archived Bot that can never write again.
use super::super::tests::fixture;
use super::*;
use agena_domain::SessionSurface;

fn create(key: &str, name: &str) -> BotCreate {
    BotCreate {
        request_key: key.into(),
        name: name.into(),
        persona: None,
    }
}

fn archive(key: &str, profile: &BotProfile) -> BotArchive {
    BotArchive {
        request_key: key.into(),
        expected_revision: profile.revision,
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
async fn concurrent_default_creation_produces_one_bot_and_one_conversation() {
    let (manager, _ordinary) = fixture().await;
    let mut calls = Vec::new();
    for _ in 0..8 {
        let manager = manager.clone();
        calls.push(tokio::spawn(async move {
            manager.ensure_bot_session().await.unwrap()
        }));
    }
    let mut ids = Vec::new();
    for call in calls {
        ids.push(call.await.unwrap());
    }
    assert!(ids.iter().all(|id| *id == ids[0]), "{ids:?}");
    let bots = manager.list_bot_profiles(false).await.unwrap();
    assert_eq!(bots.items.len(), 1);
    assert_eq!(bots.default_bot_id, Some(bots.items[0].id));
    assert_eq!(
        manager
            .session_owner(ids[0])
            .await
            .unwrap()
            .unwrap()
            .surface,
        agena_domain::SessionSurface::Local
    );
}

#[tokio::test]
async fn ownership_survives_a_restart_and_is_never_recreated_twice() {
    let (manager, _ordinary) = fixture().await;
    let session = manager.ensure_bot_session().await.unwrap();
    manager
        .rename_session(session, "User operator title".into())
        .await
        .unwrap();
    let restarted = restart(&manager);
    assert_eq!(restarted.ensure_bot_session().await.unwrap(), session);
    assert_eq!(
        restarted
            .kernel()
            .reader
            .metadata(session_id(session).unwrap())
            .await
            .unwrap()
            .title,
        "User operator title"
    );
    // Reopening keeps the one pinned local conversation.
    assert_eq!(restarted.ensure_bot_session().await.unwrap(), session);
}

#[tokio::test]
async fn names_are_unique_per_active_bot_and_revision_guards_edits() {
    let (manager, _ordinary) = fixture().await;
    let first = manager
        .create_bot_profile(create("a", "Lotus"))
        .await
        .unwrap();
    let other = manager
        .create_bot_profile(create("b", "Reed"))
        .await
        .unwrap();
    assert_ne!(first.id, other.id);

    // Case-folded duplicates are rejected only among active Bots.
    assert!(
        manager
            .create_bot_profile(create("c", " lotus "))
            .await
            .is_err()
    );
    manager
        .archive_bot_profile(other.id, archive("arch", &other))
        .await
        .unwrap();
    let reused = manager
        .create_bot_profile(create("d", "Reed"))
        .await
        .unwrap();
    assert_ne!(reused.id, other.id);

    // A stale revision cannot overwrite a newer edit.
    let updated = manager
        .update_bot_profile(
            first.id,
            BotUpdate {
                request_key: "edit".into(),
                expected_revision: first.revision,
                name: "Lotus II".into(),
                persona: BotPersona {
                    instructions: "短句优先".into(),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.revision, first.revision + 1);
    assert!(
        manager
            .update_bot_profile(
                first.id,
                BotUpdate {
                    request_key: "edit-2".into(),
                    expected_revision: first.revision,
                    name: "Lotus III".into(),
                    persona: BotPersona::default(),
                },
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn replayed_request_keys_do_not_repeat_an_effect() {
    let (manager, _ordinary) = fixture().await;
    let created = manager
        .create_bot_profile(create("once", "Nova"))
        .await
        .unwrap();
    let replay = manager
        .create_bot_profile(create("once", "Nova"))
        .await
        .unwrap();
    assert_eq!(created, replay);
    assert_eq!(
        manager.list_bot_profiles(false).await.unwrap().items.len(),
        1
    );

    // The same key with different input is refused rather than silently
    // creating a second Bot.
    assert!(
        manager
            .create_bot_profile(create("once", "Nova II"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn archived_bot_history_is_read_only_and_loses_operator_authority() {
    let (manager, _ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("arch", "Old"))
        .await
        .unwrap();
    let session = manager
        .ensure_named_bot_session(bot.id)
        .await
        .unwrap()
        .session_id;
    manager
        .archive_bot_profile(bot.id, archive("byebye", &bot))
        .await
        .unwrap();

    assert!(
        manager.require_writable_bot_history(session).await.is_err(),
        "archived history must refuse writes"
    );
    assert!(
        manager.require_operator_bot(session).await.is_err(),
        "archived history must lose cross-project authority"
    );
    assert!(
        manager.ensure_named_bot_session(bot.id).await.is_err(),
        "an archived Bot is not opened like an active one"
    );
    assert!(manager.bot_context(session).await.is_err());
}

#[tokio::test]
async fn deleting_a_bot_conversation_yields_a_fresh_one_with_the_same_identity() {
    let (manager, _ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("del", "Ephemeral"))
        .await
        .unwrap();
    let first = manager
        .ensure_named_bot_session(bot.id)
        .await
        .unwrap()
        .session_id;
    manager.delete_session(first).await.unwrap();
    let second = manager
        .ensure_named_bot_session(bot.id)
        .await
        .unwrap()
        .session_id;
    assert_ne!(
        first, second,
        "a deleted conversation must be replaced, not reused"
    );
    let configuration = manager
        .kernel()
        .reader
        .configuration(session_id(second).unwrap())
        .await
        .unwrap();
    let owners = configuration.owner.clone().unwrap();
    assert_eq!(owners.bot, bot.id);
    assert!(owners.surface.is_local());
}

#[tokio::test]
async fn archival_requires_every_owned_conversation_to_be_idle() {
    let (manager, _ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("busy", "Busy"))
        .await
        .unwrap();
    let session = session_id(
        manager
            .ensure_named_bot_session(bot.id)
            .await
            .unwrap()
            .session_id,
    )
    .unwrap();
    // Native work is enough: no supervisor has to be running for archiving to
    // see that the Bot is busy.
    manager
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
        .unwrap();
    assert!(matches!(
        manager
            .archive_bot_profile(bot.id, archive("no", &bot))
            .await,
        Err(AppError::BotConflict(_))
    ));
    assert!(!manager.get_bot_profile(bot.id).await.unwrap().archived);
}

#[tokio::test]
async fn a_bot_cannot_own_or_target_another_bots_conversation() {
    let (manager, ordinary) = fixture().await;
    let first = manager
        .create_bot_profile(create("one", "One"))
        .await
        .unwrap();
    let second = manager
        .create_bot_profile(create("two", "Two"))
        .await
        .unwrap();
    let one = manager
        .ensure_named_bot_session(first.id)
        .await
        .unwrap()
        .session_id;
    let two = manager
        .ensure_named_bot_session(second.id)
        .await
        .unwrap()
        .session_id;

    // Bot conversations are never project targets for another Bot.
    assert!(manager.require_project_session(one).await.is_err());
    assert!(manager.require_project_session(two).await.is_err());
    // An ordinary project conversation is not a Bot's operator surface.
    assert!(manager.require_operator_bot(ordinary.get()).await.is_err());
    assert!(manager.require_operator_bot(one).await.is_ok());
}

#[tokio::test]
async fn discovery_and_targeting_exclude_every_bot_conversation() {
    let (manager, ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("disc", "Hidden"))
        .await
        .unwrap();
    let session = manager
        .ensure_named_bot_session(bot.id)
        .await
        .unwrap()
        .session_id;
    let _ = ordinary;
    let ancestry = manager
        .kernel()
        .reader
        .management_ancestry(session_id(session).unwrap())
        .await
        .unwrap();
    assert!(ancestry.iter().all(|ancestor| ancestor.owner.is_some()));
    assert!(manager.is_reserved_bot_session(session).await.unwrap());
    assert!(
        !manager
            .is_reserved_bot_session(ordinary.get())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn owner_filtered_listing_returns_only_that_bots_conversations() {
    let (manager, ordinary) = fixture().await;
    let first = manager
        .create_bot_profile(create("l1", "Listed"))
        .await
        .unwrap();
    let second = manager
        .create_bot_profile(create("l2", "Other"))
        .await
        .unwrap();
    let one = manager
        .ensure_named_bot_session(first.id)
        .await
        .unwrap()
        .session_id;
    let two = manager
        .ensure_named_bot_session(second.id)
        .await
        .unwrap()
        .session_id;
    let page = manager
        .kernel()
        .reader
        .query_summaries(SummaryQuery {
            owner_bot_id: Some(first.id.to_string()),
            include_global: true,
            limit: 32,
            ..Default::default()
        })
        .await
        .unwrap();
    let ids = page
        .sessions
        .iter()
        .map(|summary| summary.id.get())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![one]);
    assert!(!ids.contains(&two));
    assert!(!ids.contains(&ordinary.get()));
}

#[tokio::test]
async fn create_and_ensure_share_one_local_conversation_and_keep_its_title() {
    let (manager, _ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("single", "Single"))
        .await
        .unwrap();
    let first = manager
        .create_bot_conversation_for(bot.id, None)
        .await
        .unwrap();
    let second = manager
        .create_bot_conversation_for(bot.id, Some("Another title".into()))
        .await
        .unwrap();
    assert_eq!(first.session_id, second.session_id);
    assert_eq!(bot.local_session_id, Some(first.session_id));
    assert_eq!(
        manager.get_session(first.session_id).await.unwrap().title,
        "Single"
    );
    let owned = manager
        .kernel()
        .reader
        .query_summaries(SummaryQuery {
            owner_bot_id: Some(bot.id.to_string()),
            include_global: true,
            limit: 32,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(owned.sessions.len(), 1);
    assert!(
        manager
            .create_bot_conversation_for(bot.id, Some("   ".into()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn concurrent_kernel_and_client_opens_after_deletion_create_only_one_new_local_root() {
    let (manager, _ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("concurrent", "Concurrent"))
        .await
        .unwrap();
    let old = bot.local_session_id.unwrap();
    manager.delete_session(old).await.unwrap();
    assert_eq!(
        manager
            .get_bot_profile(bot.id)
            .await
            .unwrap()
            .local_session_id,
        None
    );
    let mut calls = Vec::new();
    for n in 0..12 {
        let manager = manager.clone();
        let bot_id = bot.id;
        calls.push(tokio::spawn(async move {
            if n % 2 == 0 {
                // Bypass the registry mutex to exercise the database guarantee.
                persist(|| {
                    manager.kernel().sessions.ensure_local_bot_session(
                        RequestId::new(),
                        bot_id,
                        "Concurrent".into(),
                    )
                })
                .await
                .unwrap()
                .value
                .id
                .get()
            } else {
                manager
                    .create_bot_conversation_for(bot_id, None)
                    .await
                    .unwrap()
                    .session_id
            }
        }));
    }
    let mut ids = Vec::new();
    for call in calls {
        ids.push(call.await.unwrap());
    }
    assert!(ids.iter().all(|id| *id == ids[0]), "{ids:?}");
    assert_ne!(ids[0], old);
    assert_eq!(
        restart(&manager)
            .ensure_named_bot_session(bot.id)
            .await
            .unwrap()
            .session_id,
        ids[0]
    );
    let owned = manager
        .kernel()
        .reader
        .query_summaries(SummaryQuery {
            owner_bot_id: Some(bot.id.to_string()),
            owner_surface: Some("local".into()),
            roots_only: true,
            include_global: true,
            limit: 32,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(owned.sessions.len(), 1);
}

#[tokio::test]
async fn legacy_local_history_stays_readable_and_remote_rooms_remain_independent() {
    let (manager, _ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("history", "History"))
        .await
        .unwrap();
    let canonical = bot.local_session_id.unwrap();
    // Fixture for an older installation with several owned local roots.
    let legacy = manager
        .kernel()
        .sessions
        .create_global(
            RequestId::new(),
            "Old history".into(),
            SessionConfiguration {
                owner: Some(SessionOwner {
                    bot: bot.id,
                    surface: SessionSurface::Local,
                    scope_key: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    manager
        .rename_session(legacy.get(), "Preserved history".into())
        .await
        .unwrap();
    assert_eq!(
        manager.get_session(legacy.get()).await.unwrap().title,
        "Preserved history"
    );
    assert_eq!(
        manager
            .get_bot_profile(bot.id)
            .await
            .unwrap()
            .local_session_id,
        Some(canonical)
    );
    let error = manager
        .require_writable_bot_history(legacy.get())
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::BotLocalHistory { canonical_session_id } if canonical_session_id == canonical)
    );
    assert_eq!(error.failure().code.as_str(), "bot.local_history_read_only");
    assert!(
        error
            .failure()
            .user
            .fallback
            .contains(&canonical.to_string())
    );
    assert!(manager.require_bot_branch_allowed(canonical).await.is_err());

    let mut rooms = Vec::new();
    for key in ["telegram:room-a", "telegram:room-b"] {
        let room = manager
            .kernel()
            .sessions
            .create_global(
                RequestId::new(),
                key.into(),
                SessionConfiguration {
                    owner: Some(SessionOwner {
                        bot: bot.id,
                        surface: SessionSurface::Group,
                        scope_key: Some(key.into()),
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id
            .get();
        manager.require_writable_bot_history(room).await.unwrap();
        rooms.push(room);
    }
    assert_ne!(rooms[0], rooms[1]);
    manager.delete_session(canonical).await.unwrap();
    assert_eq!(
        manager
            .get_bot_profile(bot.id)
            .await
            .unwrap()
            .local_session_id,
        None
    );
    let replacement = manager
        .ensure_named_bot_session(bot.id)
        .await
        .unwrap()
        .session_id;
    assert_ne!(replacement, canonical);
    assert_ne!(
        replacement,
        legacy.get(),
        "deleting the current local must not revive old history"
    );
    assert_eq!(
        manager.get_session(legacy.get()).await.unwrap().title,
        "Preserved history"
    );
    assert!(
        manager
            .require_writable_bot_history(legacy.get())
            .await
            .is_err()
    );
    for room in rooms {
        manager.require_writable_bot_history(room).await.unwrap();
    }
}

#[tokio::test]
async fn archived_bot_cannot_start_a_conversation() {
    let (manager, _ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("dead", "Dead"))
        .await
        .unwrap();
    manager.ensure_named_bot_session(bot.id).await.unwrap();
    manager
        .archive_bot_profile(bot.id, archive("gone", &bot))
        .await
        .unwrap();
    assert!(
        manager
            .create_bot_conversation_for(bot.id, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn canonical_local_subagents_remain_writable_but_historical_branches_do_not() {
    async fn child(
        manager: &SessionManager,
        bot: Uuid,
        parent: i64,
        kind: agena_domain::SessionRelationKind,
        name: &str,
    ) -> i64 {
        manager
            .kernel()
            .sessions
            .create_global(
                RequestId::new(),
                name.into(),
                SessionConfiguration {
                    owner: Some(SessionOwner {
                        bot,
                        surface: SessionSurface::Local,
                        scope_key: None,
                    }),
                    parent_session_id: Some(session_id(parent).unwrap()),
                    relation_kind: kind,
                    agent: kind.is_subagent().then(|| SessionAgentConfiguration {
                        name: name.into(),
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
            .unwrap()
            .value
            .id
            .get()
    }
    let (manager, ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("worker-tree", "Worker tree"))
        .await
        .unwrap();
    let canonical = bot.local_session_id.unwrap();
    manager
        .require_writable_bot_history(canonical)
        .await
        .unwrap();
    manager
        .require_writable_bot_history(ordinary.get())
        .await
        .unwrap();
    manager
        .require_bot_branch_allowed(ordinary.get())
        .await
        .unwrap();
    let worker = child(
        &manager,
        bot.id,
        canonical,
        agena_domain::SessionRelationKind::Subagent,
        "worker",
    )
    .await;
    let nested = child(
        &manager,
        bot.id,
        worker,
        agena_domain::SessionRelationKind::Subagent,
        "nested",
    )
    .await;
    manager.require_writable_bot_history(worker).await.unwrap();
    manager.require_writable_bot_history(nested).await.unwrap();
    for kind in [
        agena_domain::SessionRelationKind::Child,
        agena_domain::SessionRelationKind::Fork,
        agena_domain::SessionRelationKind::Rewind,
    ] {
        let history = child(&manager, bot.id, canonical, kind, "history").await;
        assert!(
            matches!(manager.require_writable_bot_history(history).await,
            Err(AppError::BotLocalHistory { canonical_session_id }) if canonical_session_id == canonical)
        );
        assert_eq!(manager.get_session(history).await.unwrap().title, "history");
        let retained_worker = child(
            &manager,
            bot.id,
            history,
            agena_domain::SessionRelationKind::Subagent,
            "retained_worker",
        )
        .await;
        assert!(
            manager
                .require_writable_bot_history(retained_worker)
                .await
                .is_err()
        );
    }
}
