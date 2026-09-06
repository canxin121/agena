use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, DbErr,
    Statement, TransactionTrait,
};
use tracing::instrument::WithSubscriber as _;

use super::{is_sqlite_busy, run_transaction_app_effects, run_transaction_effects};

mod log_capture;

async fn initialized_database() -> (tempfile::TempDir, DatabaseConnection) {
    let fixture = tempfile::tempdir().unwrap();
    let mut options = ConnectOptions::new(format!(
        "sqlite://{}?mode=rwc",
        fixture.path().join("transaction.db").display()
    ));
    options.max_connections(1).min_connections(1);
    let db = Database::connect(options).await.unwrap();
    crate::initialize_schema(&db).await.unwrap();
    for sql in [
        "CREATE TABLE rollback_probe (id INTEGER PRIMARY KEY)",
        "CREATE TRIGGER rollback_probe_reject BEFORE INSERT ON rollback_probe WHEN NEW.id = 2 BEGIN SELECT RAISE(ROLLBACK, 'primary rollback probe rejection'); END",
    ] {
        db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
            .await
            .unwrap();
    }
    (fixture, db)
}

async fn force_automatic_rollback(transaction: &sea_orm::DatabaseTransaction) -> DbErr {
    transaction
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT INTO rollback_probe (id) VALUES (1)",
        ))
        .await
        .unwrap();
    let primary = transaction
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT INTO rollback_probe (id) VALUES (2)",
        ))
        .await
        .unwrap_err();
    assert_eq!(sqlite_code(&primary).as_deref(), Some("1811"));
    primary
}

fn sqlite_code(error: &DbErr) -> Option<String> {
    match error {
        DbErr::Exec(sea_orm::RuntimeErr::SqlxError(sea_orm::sqlx::Error::Database(error)))
        | DbErr::Query(sea_orm::RuntimeErr::SqlxError(sea_orm::sqlx::Error::Database(error))) => {
            error.code().map(|code| code.into_owned())
        }
        _ => None,
    }
}

async fn assert_rollback_and_recovery(db: &DatabaseConnection) {
    let count: i64 = db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM rollback_probe",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "count")
        .unwrap();
    assert_eq!(count, 0, "the earlier write must have rolled back");
    run_transaction_effects(db, |transaction, _effects| {
        Box::pin(async move {
            transaction
                .execute(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "INSERT INTO rollback_probe (id) VALUES (3)",
                ))
                .await?;
            Ok(())
        })
    })
    .await
    .expect("the pool can commit another transaction after rollback failure");
    let id: i64 = db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT id FROM rollback_probe",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "id")
        .unwrap();
    assert_eq!(id, 3);
}

#[tokio::test]
async fn a_failed_cleanup_preserves_the_original_sqlite_error_and_skips_effects() {
    let (_fixture, db) = initialized_database().await;
    let logs = Arc::new(log_capture::LogCapture::default());
    let ran = Arc::new(AtomicBool::new(false));
    let effect_ran = ran.clone();
    let result: Result<(), DbErr> = run_transaction_effects(&db, |transaction, effects| {
        effects.push(async move { effect_ran.store(true, Ordering::SeqCst) });
        Box::pin(async move { Err(force_automatic_rollback(transaction).await) })
    })
    .with_subscriber(logs.clone())
    .await;
    let error = result.unwrap_err();
    assert!(!ran.load(Ordering::SeqCst));
    assert_rollback_and_recovery(&db).await;
    assert_eq!(sqlite_code(&error).as_deref(), Some("1811"), "{error}");
    assert!(
        error
            .to_string()
            .contains("primary rollback probe rejection")
    );
    logs.assert_cleanup_failure_recorded();
}

#[derive(Debug, PartialEq)]
enum AppFailure {
    BusinessRule { rule: &'static str },
    Database(String),
}

impl From<DbErr> for AppFailure {
    fn from(error: DbErr) -> Self {
        Self::Database(error.to_string())
    }
}

#[tokio::test]
async fn a_failed_cleanup_preserves_the_application_error_variant_and_skips_effects() {
    let (_fixture, db) = initialized_database().await;
    let logs = Arc::new(log_capture::LogCapture::default());
    let ran = Arc::new(AtomicBool::new(false));
    let effect_ran = ran.clone();
    let result: Result<(), AppFailure> =
        run_transaction_app_effects(&db, |transaction, effects| {
            effects.push(async move { effect_ran.store(true, Ordering::SeqCst) });
            Box::pin(async move {
                let _primary = force_automatic_rollback(transaction).await;
                Err(AppFailure::BusinessRule { rule: "probe" })
            })
        })
        .with_subscriber(logs.clone())
        .await;
    assert!(!ran.load(Ordering::SeqCst));
    assert_rollback_and_recovery(&db).await;
    assert_eq!(result, Err(AppFailure::BusinessRule { rule: "probe" }));
    logs.assert_cleanup_failure_recorded();
}

#[tokio::test]
async fn sqlite_reports_517_for_an_actual_wal_snapshot_conflict() {
    let (fixture, db_a) = initialized_database().await;
    let mut options = ConnectOptions::new(format!(
        "sqlite://{}?mode=rw",
        fixture.path().join("transaction.db").display()
    ));
    options.max_connections(1).min_connections(1);
    let db_b = Database::connect(options).await.unwrap();
    let snapshot = db_a.begin().await.unwrap();
    snapshot
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM rollback_probe",
        ))
        .await
        .unwrap();
    db_b.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "INSERT INTO rollback_probe (id) VALUES (3)",
    ))
    .await
    .unwrap();
    let error = snapshot
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT INTO rollback_probe (id) VALUES (4)",
        ))
        .await
        .unwrap_err();
    snapshot.rollback().await.unwrap();
    assert_eq!(sqlite_code(&error).as_deref(), Some("517"), "{error}");
    assert!(is_sqlite_busy(&error));
}

#[test]
fn busy_detection_uses_extended_codes_before_message_fallback() {
    for code in ["5", "261", "517", "773"] {
        assert!(
            is_sqlite_busy(&super::tests::db_error_with(code, "opaque diagnostic")),
            "SQLite busy code {code} must be recognized without an English message"
        );
    }
    for code in ["1", "6", "31", "1811"] {
        assert!(
            !is_sqlite_busy(&super::tests::db_error_with(code, "database is locked")),
            "known non-busy code {code} must not be overridden by message text"
        );
    }
    assert!(is_sqlite_busy(&super::tests::db_error_with(
        "",
        "Database is locked"
    )));
}

#[tokio::test]
async fn write_lock_contention_runs_the_operation_and_effects_once_after_release() {
    use std::{sync::atomic::AtomicUsize, time::Duration};

    let (fixture, db_a) = initialized_database().await;
    let mut options = ConnectOptions::new(format!(
        "sqlite://{}?mode=rw",
        fixture.path().join("transaction.db").display()
    ));
    options.max_connections(1).min_connections(1);
    let db_b = Database::connect(options).await.unwrap();
    db_b.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "PRAGMA busy_timeout = 0",
    ))
    .await
    .unwrap();
    let locked = db_a.begin().await.unwrap();
    super::acquire_write_lock(&locked).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let operation_calls = calls.clone();
    let effect_calls = effects.clone();
    let mut task = tokio::spawn(async move {
        run_transaction_effects(&db_b, |transaction, queued| {
            operation_calls.fetch_add(1, Ordering::SeqCst);
            queued.push(async move {
                effect_calls.fetch_add(1, Ordering::SeqCst);
            });
            Box::pin(async move {
                transaction
                    .execute(Statement::from_string(
                        DatabaseBackend::Sqlite,
                        "INSERT INTO rollback_probe (id) VALUES (3)",
                    ))
                    .await?;
                Ok(())
            })
        })
        .await
    });
    let pending = tokio::time::timeout(Duration::from_millis(50), &mut task).await;
    let calls_while_locked = calls.load(Ordering::SeqCst);
    let effects_while_locked = effects.load(Ordering::SeqCst);
    // Always release the lock before assertions, including on an early error.
    locked.commit().await.unwrap();
    assert!(
        pending.is_err(),
        "the write lock still excludes the operation"
    );
    assert_eq!(calls_while_locked, 0);
    assert_eq!(effects_while_locked, 0);
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    let id: i64 = db_a
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT id FROM rollback_probe",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "id")
        .unwrap();
    assert_eq!(id, 3, "the other pool observes the committed write");
}

#[tokio::test]
async fn deferred_constraint_commit_failures_do_not_run_either_helpers_effects() {
    let (_fixture, db) = initialized_database().await;
    for sql in [
        "CREATE TABLE commit_parent (id INTEGER PRIMARY KEY)",
        "CREATE TABLE commit_child (parent_id INTEGER REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED)",
    ] {
        db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
            .await
            .unwrap();
    }
    for app_error in [false, true] {
        let ran = Arc::new(AtomicBool::new(false));
        let effect_ran = ran.clone();
        async fn operation(transaction: &sea_orm::DatabaseTransaction) -> Result<(), DbErr> {
            transaction
                .execute(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "INSERT INTO commit_child (parent_id) VALUES (42)",
                ))
                .await?;
            Ok::<(), DbErr>(())
        }
        let error = if app_error {
            run_transaction_app_effects::<(), DbErr, _>(&db, |transaction, effects| {
                effects.push(async move { effect_ran.store(true, Ordering::SeqCst) });
                Box::pin(operation(transaction))
            })
            .await
            .unwrap_err()
        } else {
            run_transaction_effects(&db, |transaction, effects| {
                effects.push(async move { effect_ran.store(true, Ordering::SeqCst) });
                Box::pin(operation(transaction))
            })
            .await
            .unwrap_err()
        };
        assert_eq!(sqlite_code(&error).as_deref(), Some("787"), "{error}");
        assert!(!ran.load(Ordering::SeqCst));
        let count: i64 = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS count FROM commit_child",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "count")
            .unwrap();
        assert_eq!(count, 0, "failed commit is rolled back before reuse");
    }
}
