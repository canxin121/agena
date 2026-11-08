use std::sync::Arc;

use agena_storage_sqlite::{SqliteTaskRepository, open_kernel_runtime};
use agena_superagent::{CreateTask, Error, Task, TaskQuery, TaskRepository, TaskState};

#[tokio::test]
async fn chatbot_inbox_binds_content_and_replays_the_original_receipt() {
    use agena_superagent::{ChatMessage, ChatReply};
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repository = SqliteTaskRepository::new(runtime.database.clone());
    let message = ChatMessage {
        session_id: 1,
        message_id: "event".into(),
        text: "hello".into(),
    };
    assert!(repository.reserve_chat(&message).await.unwrap().is_none());
    let changed = ChatMessage {
        text: "/task mutate something".into(),
        ..message.clone()
    };
    assert!(matches!(
        repository.reserve_chat(&changed).await,
        Err(Error::Conflict(_))
    ));
    let reply = ChatReply::Conversation {
        session_id: 1,
        execution_id: "original".into(),
        attempt_generation: 1,
    };
    repository.finish_chat(&message, reply).await.unwrap();
    let replacement = ChatReply::Conversation {
        session_id: 1,
        execution_id: "replacement".into(),
        attempt_generation: 2,
    };
    repository.finish_chat(&message, replacement).await.unwrap();
    let replay = repository.reserve_chat(&message).await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(replay).unwrap()["execution_id"],
        "original"
    );
}

#[tokio::test]
async fn complete_pre_task_schema_migrates_but_partial_schema_is_rejected_without_mutation() {
    use sea_orm::sqlx::Connection;
    let directory = tempfile::tempdir().unwrap();
    let source_url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("source.db").display()
    );
    let runtime = open_kernel_runtime(&source_url).await.unwrap();
    let legacy = directory.path().join("legacy.db");
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(legacy.to_string_lossy().as_ref())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    let url = format!("sqlite://{}?mode=rwc", legacy.display());
    let mut raw = sea_orm::sqlx::SqliteConnection::connect(&url)
        .await
        .unwrap();
    sea_orm::sqlx::query("DROP TABLE agena_superagent_tasks")
        .execute(&mut raw)
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
    for table in [
        "agena_assistant_profiles",
        "agena_assistant_profile_receipts",
        "agena_assistant_reminders",
        "agena_assistant_contact_budget",
    ] {
        sea_orm::sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut raw)
            .await
            .unwrap();
    }
    sea_orm::sqlx::query("DROP TABLE agena_superagent_outbox")
        .execute(&mut raw)
        .await
        .unwrap();
    sea_orm::sqlx::query("DROP TABLE agena_superagent_chat_messages")
        .execute(&mut raw)
        .await
        .unwrap();
    raw.close().await.unwrap();
    let upgraded = open_kernel_runtime(&url).await.unwrap();
    let repository = SqliteTaskRepository::new(upgraded.database.clone());
    repository.insert(task("after-upgrade", 1)).await.unwrap();

    let partial = directory.path().join("partial.db");
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(partial.to_string_lossy().as_ref())
        .execute(upgraded.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    let url = format!("sqlite://{}?mode=rwc", partial.display());
    let mut raw = sea_orm::sqlx::SqliteConnection::connect(&url)
        .await
        .unwrap();
    sea_orm::sqlx::query("DROP INDEX idx_agena_superagent_source")
        .execute(&mut raw)
        .await
        .unwrap();
    raw.close().await.unwrap();
    let before = std::fs::read(&partial).unwrap();
    assert!(open_kernel_runtime(&url).await.is_err());
    assert_eq!(std::fs::read(&partial).unwrap(), before);
}

fn task(key: &str, now: i64) -> Task {
    Task::new(
        CreateTask {
            source_session_id: 1,
            goal: "inspect the fixture".into(),
            request_key: key.into(),
        },
        1,
        now,
    )
    .unwrap()
}

#[tokio::test]
async fn concurrent_create_is_idempotent_and_changed_input_conflicts() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repository = Arc::new(SqliteTaskRepository::new(runtime.database.clone()));
    let a = task("same-event", 1);
    let b = task("same-event", 2);
    let (a, b) = tokio::join!(repository.insert(a), repository.insert(b));
    assert_eq!(a.unwrap().id, b.unwrap().id);
    assert_eq!(
        repository.list(TaskQuery::default()).await.unwrap().len(),
        1
    );
    let mut changed = task("same-event", 3);
    changed.goal = "different instruction".into();
    assert!(matches!(
        repository.insert(changed).await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn stale_revision_and_cross_scope_mutation_are_rejected() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repository = SqliteTaskRepository::new(runtime.database.clone());
    let original = repository.insert(task("one", 1)).await.unwrap();
    let mut current = original.clone();
    current.revision = 2;
    current.state = TaskState::Starting;
    repository.replace(current.clone(), 1).await.unwrap();
    assert!(matches!(
        repository.replace(current.clone(), 1).await,
        Err(Error::Conflict(_))
    ));
    current.revision = 3;
    current.workspace_id = 2;
    assert!(matches!(
        repository.replace(current, 2).await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(repository.get(original.id).await.unwrap().workspace_id, 1);
}

#[tokio::test]
async fn pending_excludes_history_and_prioritizes_active_work() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repository = SqliteTaskRepository::new(runtime.database.clone());
    let first = repository.insert(task("queued", 1)).await.unwrap();
    let mut active = task("active", 3);
    active.state = TaskState::Running;
    let active = repository.insert(active).await.unwrap();
    let mut done = task("done", 0);
    done.state = TaskState::Completed;
    repository.insert(done).await.unwrap();
    let pending = repository.pending(10).await.unwrap();
    assert_eq!(
        pending.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![active.id, first.id]
    );
    let query = TaskQuery {
        source_session_id: Some(2),
        ..Default::default()
    };
    assert!(repository.list(query).await.unwrap().is_empty());
}

#[tokio::test]
async fn persistence_survives_a_new_database_owner() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("instance.db").display()
    );
    let runtime = open_kernel_runtime(&url).await.unwrap();
    let repository = SqliteTaskRepository::new(runtime.database.clone());
    let saved = repository.insert(task("persistent", 1)).await.unwrap();
    // VACUUM captures a committed standalone database and exercises ordinary
    // owner opening, without sharing a live SQLite owner between runtimes.
    let image = directory.path().join("reopened.db");
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(image.to_string_lossy().as_ref())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    let reopened = open_kernel_runtime(&format!("sqlite://{}?mode=rwc", image.display()))
        .await
        .unwrap();
    let repository = SqliteTaskRepository::new(reopened.database.clone());
    assert_eq!(repository.get(saved.id).await.unwrap(), saved);
}

#[tokio::test]
async fn conversation_state_and_notice_commit_atomically_and_replays_freeze_payload() {
    use agena_superagent::ConversationWork;
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repository = SqliteTaskRepository::new(runtime.database.clone());
    let mut created = task("automatic-notice", 1);
    created.conversation = Some(ConversationWork::new(42, "start-intent".into(), false));
    created.worker_session_id = Some(9);
    let mut created = repository.insert(created).await.unwrap();
    assert!(
        repository
            .pending_notices(100, 32)
            .await
            .unwrap()
            .is_empty()
    );
    created.state = TaskState::Completed;
    created.revision += 1;
    created.updated_at_ms = 10;
    repository.replace(created.clone(), 1).await.unwrap();
    let notices = repository.pending_notices(100, 32).await.unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].task, created);
    assert_eq!(
        repository.for_worker(9).await.unwrap().unwrap().id,
        created.id
    );
    assert!(repository.active_workers(1).await.unwrap().is_empty());
    let key = notices[0].key.clone();
    assert_eq!(
        repository
            .prepare_notice(&key, "first verified result")
            .await
            .unwrap(),
        "first verified result"
    );
    assert_eq!(
        repository
            .prepare_notice(&key, "changed during retry")
            .await
            .unwrap(),
        "first verified result"
    );
    // A stale failed CAS cannot append any new notification.
    created.attempt = 2;
    assert!(repository.replace(created.clone(), 1).await.is_err());
    assert_eq!(repository.pending_notices(100, 32).await.unwrap().len(), 1);
    repository.defer_notice(&key, 500).await.unwrap();
    assert!(
        repository
            .pending_notices(100, 32)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repository.pending_notices(500, 32).await.unwrap()[0].failures,
        1
    );
    repository.acknowledge_notice(&key).await.unwrap();
    repository.acknowledge_notice(&key).await.unwrap();
    assert!(
        repository
            .pending_notices(1000, 32)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn complete_previous_task_baseline_upgrades_without_losing_manual_tasks() {
    use sea_orm::sqlx::Connection;
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("source.db").display()
    );
    let runtime = open_kernel_runtime(&url).await.unwrap();
    let repository = SqliteTaskRepository::new(runtime.database.clone());
    let saved = repository.insert(task("legacy-manual", 1)).await.unwrap();
    let legacy = directory.path().join("v1.db");
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(legacy.to_string_lossy().as_ref())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    let legacy_url = format!("sqlite://{}?mode=rwc", legacy.display());
    let mut raw = sea_orm::sqlx::SqliteConnection::connect(&legacy_url)
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
    for table in [
        "agena_assistant_profiles",
        "agena_assistant_profile_receipts",
        "agena_assistant_reminders",
        "agena_assistant_contact_budget",
    ] {
        sea_orm::sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut raw)
            .await
            .unwrap();
    }
    sea_orm::sqlx::query("DROP TABLE agena_superagent_outbox")
        .execute(&mut raw)
        .await
        .unwrap();
    sea_orm::sqlx::query("DROP INDEX idx_agena_superagent_worker")
        .execute(&mut raw)
        .await
        .unwrap();
    raw.close().await.unwrap();
    let upgraded = open_kernel_runtime(&legacy_url).await.unwrap();
    let repository = SqliteTaskRepository::new(upgraded.database.clone());
    assert_eq!(repository.get(saved.id).await.unwrap(), saved);
    assert!(
        repository
            .pending_notices(100, 32)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn work_delivery_read_is_current_and_scoped_even_when_enqueue_receipt_is_cached() {
    use agena_domain::{ComposerDocument, ComposerNode};
    use agena_storage::kernel::RequestId;
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/delivery-owner-test")
        .await
        .unwrap();
    let source = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Source".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let other = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Other".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let message = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            source,
            ComposerDocument(vec![ComposerNode::Text {
                text: "authorized task".into(),
            }]),
        )
        .await
        .unwrap()
        .value
        .message_id;
    let command = RequestId::new();
    let payload = serde_json::json!({"kind":"conversation_work","message_id":message.get(),"text":"verified result"});
    let original = runtime
        .ports
        .executor
        .deliver(command, source, "business-notice".into(), payload.clone())
        .await
        .unwrap()
        .value;
    assert!(!original.consumed);
    assert!(
        !runtime
            .ports
            .reader
            .delivery(source, original.id)
            .await
            .unwrap()
            .consumed
    );
    assert!(
        runtime
            .ports
            .reader
            .delivery(other, original.id)
            .await
            .is_err()
    );
    let resumed = runtime
        .ports
        .executor
        .resume_delivery(
            RequestId::new(),
            source,
            original.id,
            message,
            "verified result".into(),
        )
        .await
        .unwrap()
        .value;
    assert_eq!(
        resumed.purpose,
        agena_storage::kernel::ExecutionPurpose::Continuation
    );
    let cached = runtime
        .ports
        .executor
        .deliver(command, source, "business-notice".into(), payload)
        .await
        .unwrap()
        .value;
    assert!(
        !cached.consumed,
        "command receipts are historical, not mutable state"
    );
    let current = runtime
        .ports
        .reader
        .delivery(source, original.id)
        .await
        .unwrap();
    assert!(current.consumed);
    assert_eq!(current.id, cached.id);
}
