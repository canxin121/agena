use super::*;
use agena_domain::{ChannelAuthority, ChannelRole};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Debug)]
struct ReviewedAuthority {
    current: StdMutex<ChannelAuthority>,
    calls: AtomicUsize,
    revoke_after_first: AtomicBool,
}

#[async_trait::async_trait]
impl agena_runtime_contracts::ChannelAccessVerifier for ReviewedAuthority {
    async fn authorize(&self, authority: &ChannelAuthority) -> Result<(), String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if (self.revoke_after_first.load(Ordering::SeqCst) && call > 0)
            || authority != &*self.current.lock().unwrap()
        {
            return Err("current local channel policy does not authorize this scope".into());
        }
        Ok(())
    }
}

async fn fixture() -> (
    SessionManager,
    SessionId,
    ChannelAuthority,
    ChannelAuthority,
    Arc<ReviewedAuthority>,
    i64,
) {
    let (manager, _) = super::tests::fixture().await;
    let bot = manager.ensure_bot_session().await.unwrap();
    let mut owner = manager.session_owner(bot).await.unwrap().unwrap();
    owner.surface = agena_domain::SessionSurface::Direct;
    owner.scope_key = Some("old-epoch-1".into());
    let old = ChannelAuthority {
        binding_id: "reviewed-telegram".into(),
        epoch: 1,
        actor: "alice".into(),
        role: ChannelRole::Member,
        identity: agena_domain::ChannelIdentity {
            tenant: "telegram".into(),
            account: "bot".into(),
        },
        resource: "alice-chat".into(),
        kind: agena_domain::ConversationKind::Direct,
        thread: None,
        session_mode: agena_domain::ChannelSessionMode::PerActor,
        grant: Default::default(),
        source_admin: false,
        expires_at_ms: 0,
        memory_workspace_root: std::env::temp_dir().join("channel-recovery-alice"),
        private_paths: Default::default(),
        revoked: false,
    };
    let id = manager
        .kernel()
        .sessions
        .create_global(
            RequestId::new(),
            "Telegram history".into(),
            SessionConfiguration {
                owner: Some(owner),
                channel_authority: Some(old.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let mut current = old.clone();
    current.epoch = 3;
    current.role = ChannelRole::Owner;
    let verifier = Arc::new(ReviewedAuthority {
        current: StdMutex::new(current.clone()),
        calls: AtomicUsize::new(0),
        revoke_after_first: AtomicBool::new(false),
    });
    manager.install_channel_verifier(verifier.clone());
    (manager, id, old, current, verifier, bot)
}

async fn queued(manager: &SessionManager, id: SessionId) -> ExecutionView {
    manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            id,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Preserve this channel transcript".into(),
            }]),
        )
        .await
        .unwrap()
        .value
}

#[tokio::test]
async fn reviewed_recovery_drains_old_work_keeps_history_and_preserves_the_local_bot() {
    let (manager, id, old, current, verifier, local) = fixture().await;
    let running = queued(&manager, id).await;
    let old_attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), id, running.id)
        .await
        .unwrap()
        .value;
    let waiting = queued(&manager, id).await;
    let child = manager
        .kernel()
        .sessions
        .create_global(
            RequestId::new(),
            "old native child".into(),
            SessionConfiguration {
                parent_session_id: Some(id),
                relation_kind: agena_domain::SessionRelationKind::Subagent,
                channel_authority: Some(old.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let child_waiting = queued(&manager, child).await;
    let history = manager
        .kernel()
        .reader
        .history_after(id, None, 32)
        .await
        .unwrap()
        .entries
        .clone();

    // Represent a provider that has received cancellation but has not yet
    // released its execution lock. Recovery must wait for that release.
    let control = Arc::new(RunControl {
        session: id,
        execution: running.id,
        generation: old_attempt.generation(),
        model: ModelRef::new("test", "test"),
        state: manager.global_state.load_full(),
        cancel: CancellationToken::new(),
        phase: StdMutex::new(agena_domain::ExecutionPhase::StreamingModel),
        done: Notify::new(),
        budget: None,
        failure: Default::default(),
        settled: Default::default(),
    });
    manager
        .active
        .lock()
        .unwrap()
        .insert(running.id, control.clone());
    let dispatch = manager.session_dispatch(id);
    let executing = dispatch.execution.lock().await;
    let recovering = {
        let manager = manager.clone();
        let old = old.clone();
        let current = current.clone();
        tokio::spawn(async move {
            manager
                .recover_channel_session_authority(
                    id.get(),
                    old,
                    current,
                    "current-epoch-3",
                    || async {
                        assert!(manager.session_dispatch(id).submission.try_lock().is_err());
                        Ok(())
                    },
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), control.cancel.cancelled())
        .await
        .unwrap();
    assert!(!recovering.is_finished());
    assert_eq!(
        manager
            .kernel()
            .reader
            .configuration(id)
            .await
            .unwrap()
            .channel_authority,
        Some(old.clone())
    );
    drop(executing);
    manager.settle_supervised_run(&control);
    tokio::time::timeout(Duration::from_secs(5), recovering)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let configuration = manager.kernel().reader.configuration(id).await.unwrap();
    assert_eq!(configuration.channel_authority, Some(current.clone()));
    assert!(!configuration.message_queue_paused);
    let owner = configuration.owner.as_ref().unwrap();
    assert_eq!(owner.surface, agena_domain::SessionSurface::Direct);
    assert_eq!(owner.scope_key.as_deref(), Some("current-epoch-3"));
    assert_eq!(
        manager
            .ensure_named_bot_session(owner.bot)
            .await
            .unwrap()
            .session_id,
        local
    );
    let restored = manager
        .kernel()
        .reader
        .history_after(id, None, 32)
        .await
        .unwrap();
    assert_eq!(restored.entries.len(), history.len());
    for (before, after) in history.iter().zip(&restored.entries) {
        // Cancellation advances the view revision and execution phase, while
        // preserving the ordered transcript's identities and user payloads.
        assert_eq!(
            (
                before.id,
                before.origin_session_id,
                before.message_id,
                before.execution_id,
                before.generation,
                before.created_at_ms,
                &before.payload
            ),
            (
                after.id,
                after.origin_session_id,
                after.message_id,
                after.execution_id,
                after.generation,
                after.created_at_ms,
                &after.payload
            ),
        );
    }
    for (session, execution) in [
        (id, running.id),
        (id, waiting.id),
        (child, child_waiting.id),
    ] {
        assert_eq!(
            manager
                .kernel()
                .reader
                .execution(session, execution)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Cancelled
        );
    }
    assert!(
        old_attempt
            .begin_current_round(RequestId::new())
            .await
            .is_err()
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .configuration(child)
            .await
            .unwrap()
            .channel_authority,
        Some(old.clone())
    );
    assert!(verifier.calls.load(Ordering::SeqCst) >= 2);
    manager.verify_channel_session(id.get()).await.unwrap();

    // Reconciliation after a committed recovery must not cancel new-epoch work.
    let fresh = queued(&manager, id).await;
    manager
        .recover_channel_session_authority(id.get(), old, current, "current-epoch-3", || async {
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        manager
            .kernel()
            .reader
            .execution(id, fresh.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Queued
    );
}

#[tokio::test]
async fn review_rechecks_live_policy_after_cancelling_the_old_epoch() {
    let (manager, id, old, current, verifier, _) = fixture().await;
    let pending = queued(&manager, id).await;
    verifier.revoke_after_first.store(true, Ordering::SeqCst);
    assert!(
        manager
            .recover_channel_session_authority(
                id.get(),
                old.clone(),
                current,
                "current-epoch-3",
                || async { Ok(()) }
            )
            .await
            .is_err()
    );
    let config = manager.kernel().reader.configuration(id).await.unwrap();
    assert_eq!(config.channel_authority, Some(old));
    assert!(config.message_queue_paused);
    assert_eq!(
        manager
            .kernel()
            .reader
            .execution(id, pending.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Cancelled
    );
}

#[tokio::test]
async fn incompatible_review_does_not_cancel_or_reauthorize_existing_history() {
    let (manager, id, old, mut current, verifier, _) = fixture().await;
    let pending = queued(&manager, id).await;
    current.grant.tools.insert("fs.read".into());
    *verifier.current.lock().unwrap() = current.clone();
    assert!(
        manager
            .recover_channel_session_authority(
                id.get(),
                old.clone(),
                current,
                "current-epoch-3",
                || async { Ok(()) }
            )
            .await
            .is_err()
    );
    let config = manager.kernel().reader.configuration(id).await.unwrap();
    assert_eq!(config.channel_authority, Some(old));
    assert!(!config.message_queue_paused);
    assert_eq!(
        manager
            .kernel()
            .reader
            .execution(id, pending.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Queued
    );
}

#[tokio::test]
async fn later_review_recovers_a_compatible_kernel_epoch_after_repository_commit_failed() {
    let (manager, id, old, current, verifier, _) = fixture().await;
    let pending = queued(&manager, id).await;
    assert!(
        manager
            .recover_channel_session_authority(
                id.get(),
                old.clone(),
                current.clone(),
                "current-epoch-3",
                || async {
                    assert!(manager.session_dispatch(id).submission.try_lock().is_err());
                    Err(AppError::Config("binding repository commit failed".into()))
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .configuration(id)
            .await
            .unwrap()
            .channel_authority,
        Some(current.clone())
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .execution(id, pending.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Cancelled
    );

    let mut next = current;
    next.epoch = 5;
    *verifier.current.lock().unwrap() = next.clone();
    // The repository still remembers epoch 1. Retry must use the actual
    // compatible epoch 3 as the kernel CAS predecessor and advance to 5.
    manager
        .recover_channel_session_authority(
            id.get(),
            old,
            next.clone(),
            "current-epoch-5",
            || async { Ok(()) },
        )
        .await
        .unwrap();
    let config = manager.kernel().reader.configuration(id).await.unwrap();
    assert_eq!(config.channel_authority, Some(next));
    assert_eq!(
        config.owner.as_ref().unwrap().scope_key.as_deref(),
        Some("current-epoch-5")
    );
    assert!(!config.message_queue_paused);
}
