use agena_assistant::*;
use agena_storage_sqlite::{SqliteIntegrationRepository, open_kernel_runtime};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};

fn policy() -> ChannelAccessPolicy {
    ChannelAccessPolicy {
        identity: ChannelIdentity {
            tenant: "tenant-a".into(),
            account: "bot-a".into(),
            kind: ChannelKind::Group,
        },
        session_mode: ChannelSessionMode::PerConversation,
        thread_mode: ChannelThreadMode::PerThread,
        trigger: ChannelTrigger::DirectOrMention,
        members: vec![
            ChannelMember {
                actor: "owner".into(),
                role: ChannelRole::Owner,
                expires_at_ms: None,
            },
            ChannelMember {
                actor: "alice".into(),
                role: ChannelRole::Member,
                expires_at_ms: None,
            },
            ChannelMember {
                actor: "bob".into(),
                role: ChannelRole::Member,
                expires_at_ms: None,
            },
        ],
        allow_unlisted_chat: false,
        grant: Default::default(),
        per_user_daily_limit: 1,
        allow_automated_reports: false,
        service_grant: Default::default(),
    }
}
fn spec() -> IntegrationSpec {
    IntegrationSpec {
        title: "Room access".into(),
        provider: "test.chat".into(),
        mode: IntegrationMode::Channel,
        config: json!({}),
        resource: "room".into(),
        thread: None,
        event_types: vec!["message".into()],
        actors: vec![],
        instruction: String::new(),
        poll_seconds: 1,
        expires_at_ms: 1_000_000,
        daily_limit: 100,
        total_limit: 1000,
        max_execution_seconds: 60,
        use_source_permissions: true,
        channel_policy: Some(policy()),
    }
}
fn event(id: &str, actor: &str) -> ExternalEvent {
    ExternalEvent {
        id: id.into(),
        event_type: "message".into(),
        resource: "room".into(),
        actor: actor.into(),
        thread: Some("thread-1".into()),
        bot: false,
        occurred_at_ms: 2000,
        text: "hello".into(),
        data: Value::Null,
        channel: Some(ChannelEnvelope {
            identity: policy().identity,
            actor_verified: true,
            is_member: true,
            addressed_to_bot: true,
            verified_at_ms: 2000,
        }),
    }
}
fn authority(b: &IntegrationBinding) -> ChannelAuthority {
    ChannelAuthority {
        binding_id: b.id.to_string(),
        epoch: b.epoch,
        actor: "alice".into(),
        role: ChannelRole::Member,
        identity: policy().identity,
        resource: "room".into(),
        thread: Some("thread-1".into()),
        session_mode: ChannelSessionMode::PerConversation,
        grant: Default::default(),
        expires_at_ms: b.spec.expires_at_ms,
        memory_workspace_root: PathBuf::from("/private/alice"),
        private_paths: BTreeSet::new(),
        revoked: false,
    }
}
async fn enabled(repo: &SqliteIntegrationRepository) -> IntegrationBinding {
    let mut b = repo
        .create_binding(
            IntegrationBinding::new(1, 1, "one".into(), spec(), "fp".into(), 1000).unwrap(),
        )
        .await
        .unwrap();
    b.state = BindingState::Enabled;
    b.epoch = 1;
    b.revision = 2;
    b.activated_at_ms = Some(1500);
    repo.save_binding(b, 1).await.unwrap()
}

#[tokio::test]
async fn policy_change_is_atomic_audited_idempotent_and_blocks_old_output() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    let old = authority(&b);
    repo.save_channel_session(ChannelSessionRecord {
        binding_id: b.id,
        scope_key: "alice-thread".into(),
        session_id: 10,
        authority: old.clone(),
        created_at_ms: 2000,
    })
    .await
    .unwrap();
    repo.enqueue_delivery(IntegrationDelivery {
        id: "reply".into(),
        binding_id: b.id,
        text: "private result".into(),
        state: "queued".into(),
        attempts: 0,
        next_attempt_ms: 0,
        remote_id: None,
        created_at_ms: 2100,
        error: None,
        route: Some(ChannelOutputRoute::from_authority(&old, 10)),
    })
    .await
    .unwrap();
    let mut changed = policy();
    changed
        .members
        .iter_mut()
        .find(|m| m.actor == "alice")
        .unwrap()
        .role = ChannelRole::Blocked;
    let update = ChannelPolicyUpdate {
        expected_revision: 2,
        request_key: "revoke-alice".into(),
        policy: changed,
    };
    let next = repo
        .update_channel_policy(b.id, update.clone(), "instance_operator", 2200)
        .await
        .unwrap();
    assert!(!next.authorizes_scope(&old, 2300));
    assert_eq!(next.epoch, 2);
    assert_eq!(
        repo.update_channel_policy(b.id, update, "instance_operator", 2400)
            .await
            .unwrap()
            .revision,
        next.revision
    );
    assert!(repo.claim_delivery(b.id, 2500).await.unwrap().is_none());
    assert_eq!(repo.deliveries(b.id).await.unwrap()[0].state, "suppressed");
    let audit = repo.channel_audit(b.id).await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].actor, "instance_operator");
    assert!(
        repo.update_channel_policy(
            b.id,
            ChannelPolicyUpdate {
                expected_revision: 2,
                request_key: "stale".into(),
                policy: policy()
            },
            "instance_operator",
            2600
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn user_quota_does_not_block_other_members_and_replays_do_not_spend_again() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    repo.accept_batch(
        b.id,
        2,
        AdapterBatch {
            events: vec![event("a", "alice"), event("b", "alice"), event("c", "bob")],
            cursor: json!(1),
            retry_after_seconds: None,
        },
        3000,
    )
    .await
    .unwrap();
    let first = repo.next_event(b.id, 3100).await.unwrap().unwrap();
    assert_eq!(first.event.actor, "alice");
    assert_eq!(
        repo.next_event(b.id, 3200).await.unwrap().unwrap().event.id,
        first.event.id
    );
    let mut done = first;
    done.state = "handled".into();
    repo.finish_event(done).await.unwrap();
    let next = repo.next_event(b.id, 3300).await.unwrap().unwrap();
    assert_eq!(next.event.actor, "bob");
    assert!(
        repo.events(b.id)
            .await
            .unwrap()
            .iter()
            .any(|e| e.event.id == "b" && e.note.as_deref() == Some("per_user_daily_limit"))
    );
    assert_eq!(repo.binding(b.id).await.unwrap().admitted, 2);
}

#[tokio::test]
async fn foreign_identity_unverified_membership_and_unaddressed_group_messages_never_enter_model_queue()
 {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    let mut tenant = event("a", "alice");
    tenant.channel.as_mut().unwrap().identity.tenant = "tenant-b".into();
    let mut kind = event("b", "alice");
    kind.channel.as_mut().unwrap().identity.kind = ChannelKind::Direct;
    let mut member = event("c", "alice");
    member.channel.as_mut().unwrap().is_member = false;
    let mut mention = event("d", "alice");
    mention.channel.as_mut().unwrap().addressed_to_bot = false;
    let mut legacy = event("e", "alice");
    legacy.channel = None;
    repo.accept_batch(
        b.id,
        2,
        AdapterBatch {
            events: vec![tenant, kind, member, mention, legacy],
            cursor: json!("checked"),
            retry_after_seconds: None,
        },
        3000,
    )
    .await
    .unwrap();
    assert!(repo.next_event(b.id, 3100).await.unwrap().is_none());
    let events = repo.events(b.id).await.unwrap();
    assert_eq!(events.len(), 5);
    assert!(events.iter().all(|e| e.state == "ignored"));
    assert_eq!(repo.binding(b.id).await.unwrap().admitted, 0);
}

#[tokio::test]
async fn legacy_channel_without_policy_fails_closed_and_cannot_send_old_receipts() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let mut b = enabled(&repo).await;
    b.spec.channel_policy = None;
    repo.save_binding(b.clone(), 2).await.unwrap();
    repo.accept_batch(
        b.id,
        2,
        AdapterBatch {
            events: vec![event("legacy", "alice")],
            cursor: json!(1),
            retry_after_seconds: None,
        },
        3000,
    )
    .await
    .unwrap();
    assert!(repo.next_event(b.id, 3100).await.unwrap().is_none());
    repo.enqueue_delivery(IntegrationDelivery {
        id: "old-reply".into(),
        binding_id: b.id,
        text: "old unscoped content".into(),
        state: "queued".into(),
        attempts: 0,
        next_attempt_ms: 0,
        remote_id: None,
        created_at_ms: 2000,
        error: None,
        route: None,
    })
    .await
    .unwrap();
    assert!(repo.claim_delivery(b.id, 3200).await.unwrap().is_none());
    assert_eq!(repo.deliveries(b.id).await.unwrap()[0].state, "suppressed");
}

#[tokio::test]
async fn durable_channel_capability_cannot_be_rewritten_by_session_configuration() {
    use agena_storage::kernel::*;
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    let workspace = db.workspaces.ensure_id("/channel-immutable").await.unwrap();
    let cap = authority(&b);
    let s = db
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Scoped".into(),
            SessionConfiguration {
                channel_authority: Some(cap.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let mut escalated = cap.clone();
    escalated.grant.tools.insert("agena.fs.write".into());
    assert!(
        db.ports
            .sessions
            .configure(
                RequestId::new(),
                s,
                SessionConfigurationChange {
                    channel_authority: Some(escalated),
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        db.ports
            .reader
            .configuration(s)
            .await
            .unwrap()
            .channel_authority,
        Some(cap)
    );
}

fn platform_revocation(id: &str, actor: &str) -> ExternalEvent {
    let mut removed = event(id, actor);
    removed.event_type = "membership.revoked".into();
    removed.text = "Authenticated platform membership removal".into();
    removed.channel.as_mut().unwrap().is_member = false;
    removed.channel.as_mut().unwrap().addressed_to_bot = false;
    removed
}

#[tokio::test]
async fn verified_platform_removal_precedes_messages_and_fences_old_output_even_after_quota() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let mut b = enabled(&repo).await;
    let prior_authority = authority(&b);
    repo.enqueue_delivery(IntegrationDelivery {
        id: "before-platform-revocation".into(),
        binding_id: b.id,
        text: "must not be sent".into(),
        state: "queued".into(),
        attempts: 0,
        next_attempt_ms: 0,
        remote_id: None,
        created_at_ms: 2100,
        error: None,
        route: Some(ChannelOutputRoute::from_authority(&prior_authority, 1)),
    })
    .await
    .unwrap();
    b.admitted = b.spec.total_limit;
    repo.save_binding(b.clone(), b.revision).await.unwrap();
    let removed = platform_revocation("platform-left-alice", "alice");
    // The normal message deliberately appears FIRST. Security controls from
    // the same signed batch are applied before any new message admission.
    repo.accept_batch(
        b.id,
        b.revision,
        AdapterBatch {
            events: vec![event("same-batch-alice", "alice"), removed.clone()],
            cursor: json!({"head": 2}),
            retry_after_seconds: None,
        },
        3000,
    )
    .await
    .unwrap();
    let current = repo.binding(b.id).await.unwrap();
    assert_eq!(current.epoch, b.epoch + 1);
    assert_eq!(current.revision, b.revision + 1);
    assert_eq!(current.cursor, json!({"head": 2}));
    assert_eq!(
        current
            .spec
            .channel_policy
            .as_ref()
            .unwrap()
            .member("alice", 3100)
            .unwrap_err(),
        "member_blocked"
    );
    assert!(!current.authorizes_scope(&prior_authority, 3100));
    assert!(repo.claim_delivery(b.id, 3100).await.unwrap().is_none());
    assert_eq!(repo.deliveries(b.id).await.unwrap()[0].state, "suppressed");
    let events = repo.events(b.id).await.unwrap();
    assert_eq!(
        events
            .iter()
            .find(|e| e.event.id == "same-batch-alice")
            .unwrap()
            .state,
        "ignored"
    );
    assert_eq!(
        repo.channel_audit(b.id).await.unwrap()[0].actor,
        "platform:test.chat"
    );
    repo.accept_batch(
        b.id,
        current.revision,
        AdapterBatch {
            events: vec![removed],
            cursor: json!({"head": 3}),
            retry_after_seconds: None,
        },
        3200,
    )
    .await
    .unwrap();
    assert_eq!(repo.binding(b.id).await.unwrap().epoch, current.epoch);
    assert_eq!(repo.channel_audit(b.id).await.unwrap().len(), 1);
    assert!(repo.next_event(b.id, 3300).await.unwrap().is_none());
}

#[tokio::test]
async fn untrusted_or_historical_platform_removals_cannot_change_memberships() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    let mut foreign = platform_revocation("foreign-removal", "alice");
    foreign.channel.as_mut().unwrap().identity.tenant = "another-tenant".into();
    let mut unverified = platform_revocation("unverified-removal", "alice");
    unverified.channel.as_mut().unwrap().actor_verified = false;
    let mut historical = platform_revocation("old-removal", "owner");
    historical.occurred_at_ms = 1200;
    let mut positive = platform_revocation("fake-promote", "alice");
    positive.channel.as_mut().unwrap().is_member = true;
    positive.data = json!({"role":"owner"});
    repo.accept_batch(
        b.id,
        b.revision,
        AdapterBatch {
            events: vec![foreign, unverified, historical, positive],
            cursor: json!(4),
            retry_after_seconds: None,
        },
        3100,
    )
    .await
    .unwrap();
    let current = repo.binding(b.id).await.unwrap();
    assert_eq!(current.epoch, b.epoch);
    assert_eq!(current.spec.channel_policy, b.spec.channel_policy);
    assert!(repo.channel_audit(b.id).await.unwrap().is_empty());
    assert!(
        repo.events(b.id)
            .await
            .unwrap()
            .iter()
            .all(|row| row.state == "ignored")
    );
}

#[tokio::test]
async fn loss_of_last_platform_owner_faults_connection_instead_of_leaving_ownerless_access() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    repo.accept_batch(
        b.id,
        b.revision,
        AdapterBatch {
            events: vec![
                platform_revocation("owner-left", "owner"),
                event("bob-after-owner-left", "bob"),
            ],
            cursor: json!({"security_head": 1}),
            retry_after_seconds: None,
        },
        3000,
    )
    .await
    .unwrap();
    let current = repo.binding(b.id).await.unwrap();
    assert_eq!(current.state, BindingState::Faulted);
    assert!(current.last_error.unwrap().contains("no valid owner"));
    assert_eq!(current.cursor, json!({"security_head": 1}));
    assert!(repo.next_event(b.id, 3100).await.unwrap().is_none());
}

#[tokio::test]
async fn deterministic_access_commands_are_not_blocked_by_chat_or_execution_budgets() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let mut b = enabled(&repo).await;
    b.admitted = b.spec.total_limit;
    b.day_admitted = b.spec.daily_limit;
    repo.save_binding(b.clone(), b.revision).await.unwrap();
    let mut command = event("owner-control", "owner");
    command.text = "/access revoke alice".into();
    repo.accept_batch(
        b.id,
        b.revision,
        AdapterBatch {
            events: vec![event("ordinary-quota-blocked", "bob"), command],
            cursor: json!(2),
            retry_after_seconds: None,
        },
        3000,
    )
    .await
    .unwrap();
    let allowed = repo.next_event(b.id, 3100).await.unwrap().unwrap();
    assert_eq!(allowed.event.id, "owner-control");
    assert_eq!(repo.binding(b.id).await.unwrap().admitted, b.admitted);
}

#[tokio::test]
async fn committed_policy_receipt_replays_after_owner_expiry_without_restoring_authority() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    let mut next = b.spec.channel_policy.clone().unwrap();
    next.members
        .iter_mut()
        .find(|m| m.actor == "owner")
        .unwrap()
        .expires_at_ms = Some(5000);
    let request = ChannelPolicyUpdate {
        expected_revision: b.revision,
        request_key: "finite-owner-policy".into(),
        policy: next,
    };
    let saved = repo
        .update_channel_policy(b.id, request.clone(), "instance_operator", 3000)
        .await
        .unwrap();
    let replay = repo
        .update_channel_policy(b.id, request, "instance_operator", 6000)
        .await
        .unwrap();
    assert_eq!(saved.revision, replay.revision);
    assert!(
        replay
            .spec
            .channel_policy
            .unwrap()
            .member("owner", 6000)
            .is_err()
    );
    assert_eq!(repo.channel_audit(b.id).await.unwrap().len(), 1);
}
