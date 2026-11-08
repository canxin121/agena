use agena_storage_sqlite::{SqliteIntegrationRepository, open_kernel_runtime};
use agena_superagent::*;
use serde_json::{Value, json};

fn spec() -> IntegrationSpec {
    IntegrationSpec {
        title: "Triage".into(),
        provider: "example.github_issues".into(),
        mode: IntegrationMode::Task,
        config: json!({"repository":"owner/repo"}),
        resource: "owner/repo".into(),
        thread: None,
        event_types: vec!["issue.opened".into()],
        actors: vec![],
        instruction: "Investigate, report; do not publish".into(),
        poll_seconds: 60,
        expires_at_ms: 86400000,
        daily_limit: 2,
        total_limit: 3,
        max_task_seconds: 300,
        use_source_permissions: true,
        channel_policy: None,
    }
}
fn event(id: &str) -> ExternalEvent {
    ExternalEvent {
        id: id.into(),
        event_type: "issue.opened".into(),
        resource: "owner/repo".into(),
        actor: "contributor".into(),
        thread: None,
        bot: false,
        occurred_at_ms: 2000,
        text: "untrusted issue body".into(),
        data: Value::Null,
        channel: None,
    }
}
async fn enabled(repo: &SqliteIntegrationRepository) -> IntegrationBinding {
    let mut b = repo
        .create_binding(
            IntegrationBinding::new(
                1,
                1,
                "create".into(),
                spec(),
                "provider-fingerprint".into(),
                1000,
            )
            .unwrap(),
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
async fn inbox_checkpoint_dedupe_budget_and_revocation_are_atomic() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    repo.accept_batch(
        b.id,
        2,
        AdapterBatch {
            events: vec![event("a"), event("b"), event("c")],
            cursor: json!({"page":1}),
            retry_after_seconds: None,
        },
        3000,
    )
    .await
    .unwrap();
    let first = repo.next_event(b.id, 3100).await.unwrap().unwrap();
    assert_eq!(first.event.id, "a");
    assert_eq!(
        repo.next_event(b.id, 3200).await.unwrap().unwrap().event.id,
        "a"
    );
    assert_eq!(repo.binding(b.id).await.unwrap().admitted, 1);
    let mut done = first;
    done.state = "handled".into();
    repo.finish_event(done).await.unwrap();
    let mut changed = event("a");
    changed.text = "edited event cannot replace accepted contents".into();
    repo.accept_batch(
        b.id,
        2,
        AdapterBatch {
            events: vec![changed],
            cursor: json!({"page":2}),
            retry_after_seconds: None,
        },
        3300,
    )
    .await
    .unwrap();
    let rows = repo.events(b.id).await.unwrap();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .find(|e| e.event.id == "a")
            .unwrap()
            .event
            .text
            .contains("untrusted")
    );
    let mut second = repo.next_event(b.id, 3400).await.unwrap().unwrap();
    second.state = "handled".into();
    repo.finish_event(second).await.unwrap();
    assert!(repo.next_event(b.id, 3500).await.unwrap().is_none());
    assert_eq!(repo.binding(b.id).await.unwrap().admitted, 2);
    let mut paused = repo.binding(b.id).await.unwrap();
    paused.state = BindingState::Paused;
    paused.epoch += 1;
    paused.revision += 1;
    repo.save_binding(paused, 2).await.unwrap();
    assert!(
        repo.accept_batch(
            b.id,
            2,
            AdapterBatch {
                events: vec![event("late")],
                cursor: json!({"page":99}),
                retry_after_seconds: None
            },
            3600
        )
        .await
        .is_err()
    );
    assert_eq!(repo.binding(b.id).await.unwrap().cursor, json!({"page":2}));
    assert!(repo.next_event(b.id, 3700).await.unwrap().is_none());
}
#[tokio::test]
async fn filters_and_payload_limits_cannot_be_overridden_by_provider_data() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    let mut wrong = event("foreign");
    wrong.resource = "other/repo".into();
    let mut old = event("historical");
    old.occurred_at_ms = 1000;
    repo.accept_batch(
        b.id,
        2,
        AdapterBatch {
            events: vec![wrong, old],
            cursor: json!("head"),
            retry_after_seconds: None,
        },
        3000,
    )
    .await
    .unwrap();
    assert!(repo.next_event(b.id, 3100).await.unwrap().is_none());
    assert!(
        repo.events(b.id)
            .await
            .unwrap()
            .iter()
            .all(|e| e.state == "ignored")
    );
    let mut huge = event("too-large");
    huge.text = "x".repeat(24577);
    assert!(
        repo.accept_batch(
            b.id,
            2,
            AdapterBatch {
                events: vec![event("valid-but-rolled-back"), huge],
                cursor: json!("must-not-commit"),
                retry_after_seconds: None
            },
            3200
        )
        .await
        .is_err()
    );
    assert_eq!(repo.events(b.id).await.unwrap().len(), 2);
    assert_eq!(repo.binding(b.id).await.unwrap().cursor, json!("head"));
}
#[tokio::test]
async fn sending_recovery_never_silently_requeues_unknown_remote_effects() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = enabled(&repo).await;
    let d = IntegrationDelivery {
        id: "delivery-one".into(),
        binding_id: b.id,
        text: "hello".into(),
        state: "queued".into(),
        attempts: 0,
        next_attempt_ms: 0,
        remote_id: None,
        created_at_ms: 2000,
        error: None,
        route: None,
    };
    repo.enqueue_delivery(d.clone()).await.unwrap();
    repo.enqueue_delivery(d).await.unwrap();
    let claimed = repo.claim_delivery(b.id, 3000).await.unwrap().unwrap();
    assert_eq!(claimed.state, "sending");
    assert_eq!(claimed.attempts, 1);
    assert!(repo.claim_delivery(b.id, 3100).await.unwrap().is_none());
    repo.recover_sending().await.unwrap();
    assert_eq!(repo.deliveries(b.id).await.unwrap()[0].state, "uncertain");
    assert!(repo.claim_delivery(b.id, 4000).await.unwrap().is_none());
}
#[tokio::test]
async fn same_creation_key_cannot_rebind_identity_or_authority() {
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteIntegrationRepository::new(db.database.clone());
    let b = IntegrationBinding::new(1, 1, "creation".into(), spec(), "fp".into(), 1000).unwrap();
    let saved = repo.create_binding(b.clone()).await.unwrap();
    assert_eq!(repo.create_binding(b.clone()).await.unwrap().id, saved.id);
    let mut changed = b;
    changed.spec.resource = "other/repo".into();
    assert!(repo.create_binding(changed).await.is_err());
    assert_eq!(repo.bindings(Some(2)).await.unwrap().len(), 0);
    assert_eq!(
        repo.binding(saved.id).await.unwrap().state,
        BindingState::Disabled
    );
}
#[tokio::test]
async fn previous_assistant_baseline_upgrades_without_erasing_profiles() {
    use sea_orm::sqlx::Connection;
    let temp = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        temp.path().join("current.db").display()
    );
    let db = open_kernel_runtime(&url).await.unwrap();
    let old = temp.path().join("old.db");
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(old.to_string_lossy().as_ref())
        .execute(db.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    let url = format!("sqlite://{}?mode=rwc", old.display());
    let mut raw = sea_orm::sqlx::SqliteConnection::connect(&url)
        .await
        .unwrap();
    for table in [
        "agena_channel_access_audit",
        "agena_channel_user_budget",
        "agena_channel_sessions",
    ] {
        sea_orm::sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut raw)
            .await
            .unwrap();
    }
    for table in [
        "agena_integration_inbox",
        "agena_integration_outbox",
        "agena_integration_bindings",
    ] {
        sea_orm::sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut raw)
            .await
            .unwrap();
    }
    raw.close().await.unwrap();
    let upgraded = open_kernel_runtime(&url).await.unwrap();
    let repo = SqliteIntegrationRepository::new(upgraded.database.clone());
    assert!(repo.bindings(Some(1)).await.unwrap().is_empty());
}

#[tokio::test]
async fn empty_source_can_receive_authorized_system_notice_without_fake_user_input() {
    use agena_storage::kernel::*;
    let db = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = db
        .workspaces
        .ensure_id("/integration-system-origin")
        .await
        .unwrap();
    let session = db
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "System source".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let payload = json!({"kind":"conversation_work","work_id":"external-result","text":"authorized event result"});
    let receipt = db
        .ports
        .executor
        .deliver_control_notice(
            RequestId::new(),
            session,
            "event-notice".into(),
            payload.clone(),
            "authorized event result".into(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.value.purpose, ExecutionPurpose::Continuation);
    let replay = db
        .ports
        .executor
        .deliver_control_notice(
            RequestId::new(),
            session,
            "event-notice".into(),
            payload,
            "authorized event result".into(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.value.id, replay.value.id);
    let rows = db.ports.reader.history(session, None, 50).await.unwrap();
    assert!(
        rows.entries
            .iter()
            .all(|entry| !matches!(entry.payload, EntryPayload::User { .. }))
    );
}
