use agena_domain::{
    ChannelAuthority, ChannelGrant, ChannelIdentity, ChannelRole, ChannelSessionMode,
    ComposerDocument, ComposerNode, ConversationKind, SessionRelationKind,
};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};

fn authority() -> ChannelAuthority {
    ChannelAuthority {
        platform: String::new(),
        group_ids: Vec::new(),
        profile_ids: Vec::new(),
        profile_fingerprint: String::new(),
        permission_overlay: Default::default(),
        source_workspace_root: None,
        binding_id: "reviewed-channel".into(),
        epoch: 1,
        actor: "alice".into(),
        role: ChannelRole::Member,
        identity: ChannelIdentity {
            tenant: "telegram".into(),
            account: "bot".into(),
        },
        resource: "private-chat".into(),
        kind: ConversationKind::Direct,
        thread: None,
        session_mode: ChannelSessionMode::PerActor,
        grant: ChannelGrant::default(),
        source_admin: false,
        expires_at_ms: 0,
        memory_workspace_root: "/private/alice".into(),
        private_paths: ["/private/alice".into()].into(),
        revoked: false,
    }
}

async fn fixture() -> (SqliteKernel, SessionId) {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let id = runtime
        .ports
        .sessions
        .create_global(
            RequestId::new(),
            "Telegram history".into(),
            SessionConfiguration {
                channel_authority: Some(authority()),
                message_queue_paused: true,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    (runtime, id)
}

async fn recovery(runtime: &SqliteKernel, id: SessionId) -> ChannelAuthorityRecovery {
    let snapshot = runtime.ports.reader.control_snapshot(id, 1).await.unwrap();
    let expected = snapshot.configuration.channel_authority.clone().unwrap();
    let mut current = expected.clone();
    current.epoch = 3;
    current.role = ChannelRole::Owner;
    ChannelAuthorityRecovery {
        metadata_version: snapshot.metadata.metadata_version.get(),
        control_version: snapshot.control_version,
        expected,
        current,
        scope_key: "reviewed-epoch-3".into(),
    }
}

async fn send(runtime: &SqliteKernel, id: SessionId) -> ExecutionView {
    runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            id,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Keep this conversation history".into(),
            }]),
        )
        .await
        .unwrap()
        .value
}

#[tokio::test]
async fn compatible_recovery_preserves_history_and_resumes_only_new_input() {
    let (runtime, id) = fixture().await;
    let queued = send(&runtime, id).await;
    runtime
        .ports
        .sessions
        .cancel_owned(RequestId::new(), id, queued.id)
        .await
        .unwrap();
    let before = runtime
        .ports
        .reader
        .history_after(id, None, 32)
        .await
        .unwrap();
    let recover = recovery(&runtime, id).await;
    let configured = runtime
        .ports
        .sessions
        .recover_channel_authority(RequestId::new(), id, recover.clone())
        .await
        .unwrap()
        .value;
    assert_eq!(configured.channel_authority, Some(recover.current));
    assert!(!configured.message_queue_paused);
    let after = runtime
        .ports
        .reader
        .history_after(id, None, 32)
        .await
        .unwrap();
    assert_eq!(before.entries, after.entries);
    assert_eq!(
        runtime
            .ports
            .reader
            .execution(id, queued.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Cancelled
    );
    let next = send(&runtime, id).await;
    runtime
        .ports
        .executor
        .claim_next(RequestId::new(), id, next.id)
        .await
        .unwrap();
}

#[tokio::test]
async fn recovery_rejects_scope_capability_expiry_and_epoch_changes() {
    let (runtime, id) = fixture().await;
    let valid = recovery(&runtime, id).await;
    let changes: &[fn(&mut ChannelAuthority)] = &[
        |a| a.epoch = 1,
        |a| a.epoch = 0,
        |a| a.binding_id = "other-binding".into(),
        |a| a.actor = "bob".into(),
        |a| a.identity.account = "other-bot".into(),
        |a| a.resource = "other-room".into(),
        |a| a.kind = ConversationKind::Group,
        |a| a.thread = Some("other-thread".into()),
        |a| a.session_mode = ChannelSessionMode::PerConversation,
        |a| {
            a.grant.tools.insert("fs.read".into());
        },
        |a| a.source_admin = true,
        |a| a.expires_at_ms = i64::MAX,
        |a| a.expires_at_ms = 1,
        |a| a.memory_workspace_root = "/other-memory".into(),
        |a| {
            a.private_paths.insert("/other-path".into());
        },
        |a| a.role = ChannelRole::Blocked,
        |a| a.revoked = true,
    ];
    for change in changes {
        let mut changed = valid.clone();
        change(&mut changed.current);
        assert!(
            runtime
                .ports
                .sessions
                .recover_channel_authority(RequestId::new(), id, changed)
                .await
                .is_err()
        );
    }
    let mut stale_metadata = valid.clone();
    stale_metadata.metadata_version += 1;
    assert!(
        runtime
            .ports
            .sessions
            .recover_channel_authority(RequestId::new(), id, stale_metadata)
            .await
            .is_err()
    );
    let mut stale_control = valid.clone();
    stale_control.control_version += 1;
    assert!(
        runtime
            .ports
            .sessions
            .recover_channel_authority(RequestId::new(), id, stale_control)
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(id)
            .await
            .unwrap()
            .channel_authority,
        Some(valid.expected)
    );
}

#[tokio::test]
async fn recovery_requires_old_root_and_native_child_executions_to_be_terminal() {
    let (runtime, id) = fixture().await;
    let queued = send(&runtime, id).await;
    assert!(
        runtime
            .ports
            .sessions
            .recover_channel_authority(RequestId::new(), id, recovery(&runtime, id).await)
            .await
            .is_err()
    );
    runtime
        .ports
        .sessions
        .cancel_owned(RequestId::new(), id, queued.id)
        .await
        .unwrap();
    let child = runtime
        .ports
        .sessions
        .create_global(
            RequestId::new(),
            "old child".into(),
            SessionConfiguration {
                parent_session_id: Some(id),
                relation_kind: SessionRelationKind::Subagent,
                channel_authority: Some(authority()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let child_execution = send(&runtime, child).await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), child, child_execution.id)
        .await
        .unwrap()
        .value;
    assert!(
        runtime
            .ports
            .sessions
            .recover_channel_authority(RequestId::new(), id, recovery(&runtime, id).await)
            .await
            .is_err()
    );
    runtime
        .ports
        .sessions
        .cancel_owned(RequestId::new(), child, child_execution.id)
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .recover_channel_authority(RequestId::new(), id, recovery(&runtime, id).await)
        .await
        .unwrap();
    assert!(attempt.begin_current_round(RequestId::new()).await.is_err());
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(child)
            .await
            .unwrap()
            .channel_authority,
        Some(authority())
    );
}

#[tokio::test]
async fn ordinary_configuration_cannot_bypass_reviewed_recovery() {
    let (runtime, id) = fixture().await;
    let current = recovery(&runtime, id).await.current;
    assert!(
        runtime
            .ports
            .sessions
            .configure(
                RequestId::new(),
                id,
                SessionConfigurationChange {
                    channel_authority: Some(current),
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(id)
            .await
            .unwrap()
            .channel_authority,
        Some(authority())
    );
}
