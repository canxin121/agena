//! Bot identity table and conversation-ownership tests.
//!
//! The table is an ordinary SQLite relation now, so these tests cover the
//! properties that made the old JSON registry correct: one winner under
//! concurrency, idempotent request keys, revision compare-and-set, ownership
//! recorded on conversations, and an archived Bot that can never write again.
use super::super::tests::fixture;
use super::*;

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
    // A Bot may own more than one local conversation; opening it twice does not
    // create a second one.
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
async fn a_bot_can_start_several_independent_local_conversations() {
    let (manager, _ordinary) = fixture().await;
    let bot = manager
        .create_bot_profile(create("multi", "Multi"))
        .await
        .unwrap();
    let first = manager
        .create_bot_conversation_for(bot.id, None)
        .await
        .unwrap();
    let second = manager
        .create_bot_conversation_for(bot.id, Some("Second topic".into()))
        .await
        .unwrap();
    assert_ne!(first.session_id, second.session_id);
    assert_eq!(second.name, "Multi");

    // Both belong to the same Bot and are local roots.
    for session in [first.session_id, second.session_id] {
        let owner = manager.session_owner(session).await.unwrap().unwrap();
        assert_eq!(owner.bot, bot.id);
        assert!(owner.surface.is_local());
        assert!(manager.require_operator_bot(session).await.is_ok());
    }

    // Creating a Bot already starts one local conversation; the two above are
    // additions, so the Bot now owns three.
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
        .unwrap()
        .sessions
        .to_vec();
    assert_eq!(owned.len(), 3);
    let ids = owned.iter().map(|s| s.id.get()).collect::<Vec<_>>();
    assert!(ids.contains(&first.session_id) && ids.contains(&second.session_id));

    // Reopening the Bot reuses one of them instead of adding a fourth.
    let reopened = manager.ensure_named_bot_session(bot.id).await.unwrap();
    assert_eq!(reopened.session_id, ids[0]);

    // A title is user-supplied and validated.
    assert!(
        manager
            .create_bot_conversation_for(bot.id, Some("   ".into()))
            .await
            .is_err()
    );
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
