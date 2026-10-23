use agena_scheduler::{JobStore, ScheduledJob, SqliteJobStore};
use agena_storage::kernel::RequestId;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use crate::open_kernel_runtime;

#[tokio::test]
async fn sessions_scheduler_and_application_state_share_one_owned_database() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("instance").join("agena.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let kernel = open_kernel_runtime(&url).await.unwrap();
    let identity = kernel.instance_id.clone();
    let workspace = kernel
        .workspaces
        .ensure_id("/unified-storage-test")
        .await
        .unwrap();
    let scheduler = SqliteJobStore::new(kernel.database.as_ref().clone());
    let job = ScheduledJob::new_once(
        chrono::Utc::now() + chrono::Duration::hours(1),
        "durable reminder",
    );
    let job_id = job.id;

    let (session, scheduled, stored) = tokio::join!(
        kernel
            .ports
            .sessions
            .create(RequestId::new(), workspace, "one instance".into()),
        scheduler.put(job),
        kernel
            .instance_state
            .set_payload("terminal.registry", "{\"version\":1}".into()),
    );
    assert!(session.unwrap().value.id.get() > 0);
    scheduled.unwrap();
    stored.unwrap();
    assert_eq!(kernel.instance_state.path(), path.canonicalize().unwrap());
    assert!(scheduler.get(job_id).await.unwrap().is_some());
    assert!(
        open_kernel_runtime(&url).await.is_err(),
        "the instance has exactly one owner"
    );
    let removed_tables = kernel.database.query_all(Statement::from_string(
        DatabaseBackend::Sqlite,
        "SELECT name FROM sqlite_schema WHERE name IN ('server_kv','attachment_cache_blob_store','attachment_cache_source_index')",
    )).await.unwrap();
    assert!(removed_tables.is_empty());

    kernel.ports.clone().shutdown().await.unwrap();
    drop(scheduler);
    drop(kernel);

    let reopened = open_kernel_runtime(&url).await.unwrap();
    assert_eq!(reopened.instance_id, identity);
    assert_eq!(
        reopened
            .instance_state
            .get_payload("terminal.registry")
            .await
            .unwrap()
            .as_deref(),
        Some("{\"version\":1}")
    );
    let scheduler = SqliteJobStore::new(reopened.database.as_ref().clone());
    assert!(scheduler.get(job_id).await.unwrap().is_some());
    let sessions = reopened
        .database
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM agena_sessions",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(sessions, 1);
    reopened.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn independent_memory_instances_have_distinct_identity_and_state() {
    let first = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let second = open_kernel_runtime("sqlite::memory:").await.unwrap();
    assert_ne!(first.instance_id, second.instance_id);
    assert_ne!(first.instance_state.path(), second.instance_state.path());
    first
        .instance_state
        .set_payload("example", "{\"value\":1}".into())
        .await
        .unwrap();
    assert!(
        second
            .instance_state
            .get_payload("example")
            .await
            .unwrap()
            .is_none()
    );
    first.ports.shutdown().await.unwrap();
    second.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn state_json_is_constrained_and_identical_writes_do_not_change_timestamps() {
    let kernel = open_kernel_runtime("sqlite::memory:").await.unwrap();
    kernel
        .instance_state
        .set_payload(" stable ", "{\"ok\":true}".into())
        .await
        .unwrap();
    kernel
        .database
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "UPDATE agena_server_state SET updated_at_ms=7 WHERE key='stable'",
        ))
        .await
        .unwrap();
    kernel
        .instance_state
        .set_payload("stable", "{\"ok\":true}".into())
        .await
        .unwrap();
    let timestamp = kernel
        .database
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT updated_at_ms FROM agena_server_state WHERE key='stable'",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "updated_at_ms")
        .unwrap();
    assert_eq!(timestamp, 7);
    assert!(
        kernel
            .instance_state
            .set_payload("stable", "invalid JSON".into())
            .await
            .is_err()
    );
    assert!(
        kernel
            .instance_state
            .set_payload(" ", "null".into())
            .await
            .is_err()
    );
    assert_eq!(
        kernel
            .instance_state
            .get_payload("stable")
            .await
            .unwrap()
            .as_deref(),
        Some("{\"ok\":true}")
    );
    kernel.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn missing_instance_identity_is_rejected_without_repair() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("agena.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let kernel = open_kernel_runtime(&url).await.unwrap();
    kernel
        .database
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "DELETE FROM agena_instance",
        ))
        .await
        .unwrap();
    kernel.ports.clone().shutdown().await.unwrap();
    drop(kernel);
    let before = std::fs::read(&path).unwrap();
    let error = open_kernel_runtime(&url)
        .await
        .err()
        .expect("missing identity is invalid");
    assert!(error.to_string().contains("instance identity is missing"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[cfg(unix)]
#[tokio::test]
async fn database_sidecars_and_lock_are_private_without_changing_shared_ancestors() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempfile::tempdir().unwrap();
    std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let directory = fixture.path().join("instance");
    let path = directory.join("agena.db");
    let kernel = open_kernel_runtime(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    kernel
        .instance_state
        .set_payload("private", "true".into())
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(fixture.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for suffix in ["", "-wal", "-shm", ".owner-lock"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        assert_eq!(
            std::fs::metadata(std::path::Path::new(&file))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "{file:?}"
        );
    }
    kernel.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn opening_without_creation_permission_does_not_create_a_database() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("missing.db");
    assert!(
        open_kernel_runtime(&format!("sqlite://{}?mode=rw", path.display()))
            .await
            .is_err()
    );
    assert!(!path.exists());
}
