use super::KernelDatabase;
use std::path::{Path, PathBuf};

use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, SqlxSqliteConnector, Statement,
    TransactionTrait, TryGetable,
    sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

fn url(path: &Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

fn sqlite_file(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

fn sqlite_bytes(path: &Path) -> Vec<Option<Vec<u8>>> {
    ["", "-wal", "-shm", "-journal"]
        .into_iter()
        .map(|suffix| match std::fs::read(sqlite_file(path, suffix)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("read fixture {suffix}: {error}"),
        })
        .collect()
}

async fn raw_fixture(path: &Path) -> DatabaseConnection {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
                .foreign_keys(true)
                .journal_mode(SqliteJournalMode::Delete),
        )
        .await
        .unwrap();
    SqlxSqliteConnector::from_sqlx_sqlite_pool(pool)
}

async fn execute(db: &impl ConnectionTrait, sql: &str) {
    db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
        .await
        .unwrap();
}

async fn scalar<T: TryGetable>(db: &impl ConnectionTrait, sql: &str, column: &str) -> T {
    db.query_one(Statement::from_string(DatabaseBackend::Sqlite, sql))
        .await
        .unwrap()
        .unwrap()
        .try_get("", column)
        .unwrap()
}

async fn assert_rejected_unchanged(path: &Path) {
    let before = sqlite_bytes(path);
    // A rejected open must also release ownership, allowing another attempt.
    for _ in 0..2 {
        let error = match KernelDatabase::open(&url(path)).await {
            Ok(database) => {
                database.close().await.unwrap();
                panic!("incompatible fixture was accepted")
            }
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("new empty data directory"),
            "{error}"
        );
        assert_eq!(sqlite_bytes(path), before);
    }
}

#[tokio::test]
async fn fresh_baseline_reopens_with_identity_data_and_transaction_guards() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("instance.db");
    let database = KernelDatabase::open(&url(&path)).await.unwrap();
    let db = database.writer.as_ref();
    let identity = crate::kernel::schema::instance_id(db).await.unwrap();
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT schema_baseline FROM agena_instance",
            "schema_baseline"
        )
        .await,
        crate::schema::SCHEMA_BASELINE
    );
    assert_eq!(
        scalar::<String>(db, "PRAGMA journal_mode", "journal_mode").await,
        "wal"
    );
    assert_eq!(
        scalar::<i64>(db, "PRAGMA foreign_keys", "foreign_keys").await,
        1
    );
    assert_eq!(
        scalar::<i64>(db, "PRAGMA synchronous", "synchronous").await,
        2
    );
    assert!(
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "UPDATE agena_instance SET schema_baseline='another-baseline'",
        ))
        .await
        .is_err()
    );

    let transaction = db.begin().await.unwrap();
    execute(
        &transaction,
        "INSERT INTO agena_workspaces(path,created_at_ms,updated_at_ms) VALUES('/rolled-back',1,1)",
    )
    .await;
    assert!(transaction.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "INSERT INTO agena_sessions(workspace_id,title,created_at_ms,updated_at_ms) VALUES(999,'invalid reference',1,1)",
    )).await.is_err());
    transaction.rollback().await.unwrap();
    assert_eq!(
        scalar::<i64>(db, "SELECT count(*) AS n FROM agena_workspaces", "n").await,
        0
    );
    execute(
        db,
        "INSERT INTO agena_server_state VALUES('fixture', '{\"ready\":true}', 1)",
    )
    .await;
    database.close().await.unwrap();

    let reopened = KernelDatabase::open(&url(&path)).await.unwrap();
    assert_eq!(
        crate::kernel::schema::instance_id(reopened.writer.as_ref())
            .await
            .unwrap(),
        identity
    );
    assert_eq!(
        scalar::<String>(
            reopened.writer.as_ref(),
            "SELECT value_json FROM agena_server_state WHERE key='fixture'",
            "value_json"
        )
        .await,
        r#"{"ready":true}"#
    );
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn memory_instances_have_the_current_baseline_and_independent_identities() {
    let first = KernelDatabase::open("sqlite::memory:").await.unwrap();
    let second = KernelDatabase::open("sqlite::memory:").await.unwrap();
    assert_ne!(
        crate::kernel::schema::instance_id(first.writer.as_ref())
            .await
            .unwrap(),
        crate::kernel::schema::instance_id(second.writer.as_ref())
            .await
            .unwrap(),
    );
    first.close().await.unwrap();
    second.close().await.unwrap();
}

#[tokio::test]
async fn prior_database_is_rejected_without_creating_a_wal_or_changing_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prior.db");
    let fixture = raw_fixture(&path).await;
    execute(&fixture, "CREATE TABLE prior_records(value TEXT NOT NULL)").await;
    execute(
        &fixture,
        "INSERT INTO prior_records VALUES('keep this record')",
    )
    .await;
    fixture.close().await.unwrap();
    assert_rejected_unchanged(&path).await;
    assert!(!sqlite_file(&path, "-wal").exists());
    assert!(!sqlite_file(&path, "-shm").exists());
    let fixture = raw_fixture(&path).await;
    assert_eq!(
        scalar::<String>(&fixture, "SELECT value FROM prior_records", "value").await,
        "keep this record"
    );
    fixture.close().await.unwrap();
}

#[tokio::test]
async fn pre_baseline_instance_with_otherwise_current_schema_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prior-instance.db");
    KernelDatabase::open(&url(&path))
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
    let fixture = raw_fixture(&path).await;
    execute(&fixture, "DROP TABLE agena_instance").await;
    // One historical table shape is enough to represent a pre-baseline file;
    // every other durable declaration remains the current full schema.
    execute(&fixture, "CREATE TABLE agena_instance (id INTEGER PRIMARY KEY CHECK(id=1), instance_id TEXT NOT NULL UNIQUE CHECK(length(instance_id)=36), created_at_ms INTEGER NOT NULL)").await;
    execute(
        &fixture,
        "INSERT INTO agena_instance VALUES(1,'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',1)",
    )
    .await;
    fixture.close().await.unwrap();
    assert_rejected_unchanged(&path).await;
}

#[tokio::test]
async fn changed_baseline_value_and_missing_identity_are_rejected_without_repair() {
    for mutation in [
        "UPDATE agena_instance SET schema_baseline='prior-baseline'",
        "DELETE FROM agena_instance",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.db");
        KernelDatabase::open(&url(&path))
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        let fixture = raw_fixture(&path).await;
        execute(&fixture, "PRAGMA ignore_check_constraints=ON").await;
        execute(&fixture, mutation).await;
        execute(&fixture, "PRAGMA ignore_check_constraints=OFF").await;
        fixture.close().await.unwrap();
        assert_rejected_unchanged(&path).await;
    }
}

#[tokio::test]
async fn missing_schema_object_is_rejected_instead_of_reinstalled() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("schema-drift.db");
    KernelDatabase::open(&url(&path))
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
    let fixture = raw_fixture(&path).await;
    execute(&fixture, "DROP INDEX idx_agena_server_state_updated").await;
    fixture.close().await.unwrap();
    assert_rejected_unchanged(&path).await;
}

#[tokio::test]
async fn prior_database_with_active_wal_preserves_all_sqlite_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prior-wal.db");
    let fixture = raw_fixture(&path).await;
    execute(&fixture, "CREATE TABLE prior_records(value TEXT NOT NULL)").await;
    execute(&fixture, "INSERT INTO prior_records VALUES('checkpointed')").await;
    execute(&fixture, "PRAGMA journal_mode=WAL").await;
    execute(&fixture, "PRAGMA wal_autocheckpoint=0").await;
    execute(&fixture, "INSERT INTO prior_records VALUES('in the WAL')").await;
    assert!(sqlite_file(&path, "-wal").metadata().unwrap().len() > 32);
    assert_rejected_unchanged(&path).await;
    fixture.close().await.unwrap();
}

#[tokio::test]
async fn concurrent_opens_keep_one_service_owner_and_release_it_on_close() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("concurrent.db");
    let url = url(&path);
    let (first, second) = tokio::join!(KernelDatabase::open(&url), KernelDatabase::open(&url));
    let (owner, error) = match (first, second) {
        (Ok(owner), Err(error)) | (Err(error), Ok(owner)) => (owner, error),
        _ => panic!("concurrent opens must admit exactly one owner"),
    };
    assert!(error.to_string().contains("already has a service owner"));
    owner.close().await.unwrap();
    KernelDatabase::open(&url)
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

fn crash_fixture(path: &Path, kind: &str) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "connection::tests::crash_fixture_child",
            "--ignored",
            "--nocapture",
        ])
        .env("AGENA_SQLITE_CRASH_FIXTURE_PATH", path)
        .env("AGENA_SQLITE_CRASH_FIXTURE_KIND", kind)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "subprocess fixture invoked by the crash recovery tests"]
async fn crash_fixture_child() {
    let path = PathBuf::from(std::env::var_os("AGENA_SQLITE_CRASH_FIXTURE_PATH").unwrap());
    let kind = std::env::var("AGENA_SQLITE_CRASH_FIXTURE_KIND").unwrap();
    match kind.as_str() {
        "current-wal" => {
            let database = KernelDatabase::open(&url(&path)).await.unwrap();
            let db = database.writer.as_ref();
            execute(db, "PRAGMA wal_autocheckpoint=0").await;
            execute(
                db,
                "INSERT INTO agena_server_state VALUES('committed', '{\"durable\":true}', 1)",
            )
            .await;
            let transaction = db.begin().await.unwrap();
            execute(
                &transaction,
                "INSERT INTO agena_server_state VALUES('uncommitted', '{}', 2)",
            )
            .await;
            // No pool, transaction or connection destructor may checkpoint the
            // WAL or roll back the unfinished transaction before process exit.
            std::process::exit(0);
        }
        "current-wal-schema-drift" | "current-wal-identity-drift" => {
            let database = KernelDatabase::open(&url(&path)).await.unwrap();
            let db = database.writer.as_ref();
            execute(db, "PRAGMA wal_autocheckpoint=0").await;
            execute(
                db,
                "INSERT INTO agena_server_state VALUES('keep', '{\"durable\":true}', 1)",
            )
            .await;
            if kind == "current-wal-schema-drift" {
                execute(db, "DROP INDEX idx_agena_server_state_updated").await;
            } else {
                execute(db, "PRAGMA ignore_check_constraints=ON").await;
                execute(
                    db,
                    "UPDATE agena_instance SET schema_baseline='another-baseline'",
                )
                .await;
                execute(db, "PRAGMA ignore_check_constraints=OFF").await;
            }
            // Leave the schema or identity change committed only in WAL. The
            // main file still passes an immutable current-baseline inspection.
            std::process::exit(0);
        }
        "prior-journal" => {
            let fixture = raw_fixture(&path).await;
            execute(&fixture, "PRAGMA synchronous=FULL").await;
            execute(&fixture, "PRAGMA cache_size=1").await;
            execute(
                &fixture,
                "CREATE TABLE prior_records(value TEXT NOT NULL, payload BLOB NOT NULL)",
            )
            .await;
            execute(
                &fixture,
                "INSERT INTO prior_records VALUES('kept',zeroblob(65536))",
            )
            .await;
            let transaction = fixture.begin().await.unwrap();
            execute(
                &transaction,
                "UPDATE prior_records SET value='uncommitted',payload=randomblob(65536)",
            )
            .await;
            // Dirty page eviction creates a hot rollback journal. A normal
            // read-write open would recover it before schema inspection.
            std::process::exit(0);
        }
        other => panic!("unknown crash fixture {other}"),
    }
}

#[tokio::test]
async fn current_baseline_recovers_committed_uncheckpointed_wal_after_a_crash() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recover.db");
    crash_fixture(&path, "current-wal");
    assert!(sqlite_file(&path, "-wal").metadata().unwrap().len() > 32);

    let main_only = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .read_only(true)
                .immutable(true),
        )
        .await
        .unwrap();
    let main_only = SqlxSqliteConnector::from_sqlx_sqlite_pool(main_only);
    crate::kernel::schema::validate_existing(&main_only)
        .await
        .unwrap();
    assert_eq!(
        scalar::<i64>(
            &main_only,
            "SELECT count(*) AS n FROM agena_server_state",
            "n"
        )
        .await,
        0
    );
    main_only.close().await.unwrap();

    let database = KernelDatabase::open(&url(&path)).await.unwrap();
    assert_eq!(
        scalar::<String>(
            database.writer.as_ref(),
            "SELECT value_json FROM agena_server_state WHERE key='committed'",
            "value_json"
        )
        .await,
        r#"{"durable":true}"#
    );
    assert_eq!(
        scalar::<i64>(
            database.writer.as_ref(),
            "SELECT count(*) AS n FROM agena_server_state WHERE key='uncommitted'",
            "n"
        )
        .await,
        0
    );
    database.close().await.unwrap();
}

#[tokio::test]
async fn prior_hot_journal_is_rejected_without_triggering_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prior-crash.db");
    crash_fixture(&path, "prior-journal");
    assert!(sqlite_file(&path, "-journal").metadata().unwrap().len() > 512);
    assert_rejected_unchanged(&path).await;
    // This explicit fixture open, after the preservation assertion, performs
    // recovery and proves that the retained journal represented durable data.
    let fixture = raw_fixture(&path).await;
    assert_eq!(
        scalar::<String>(&fixture, "SELECT value FROM prior_records", "value").await,
        "kept"
    );
    fixture.close().await.unwrap();
}

#[tokio::test]
async fn wal_only_schema_and_identity_drift_are_rejected_without_source_writes() {
    for kind in ["current-wal-schema-drift", "current-wal-identity-drift"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wal-drift.db");
        crash_fixture(&path, kind);
        assert!(sqlite_file(&path, "-wal").metadata().unwrap().len() > 32);
        let main_only = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .read_only(true)
                    .immutable(true),
            )
            .await
            .unwrap();
        let main_only = SqlxSqliteConnector::from_sqlx_sqlite_pool(main_only);
        // This proves the fixture reaches recovered-view admission: checking
        // only the immutable main file cannot detect either incompatible WAL.
        crate::kernel::schema::validate_existing(&main_only)
            .await
            .unwrap();
        main_only.close().await.unwrap();

        let entries = || {
            std::fs::read_dir(directory.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before_entries = entries();
        assert_rejected_unchanged(&path).await;
        assert_eq!(
            entries(),
            before_entries,
            "temporary admission snapshots must be cleaned up"
        );
    }
}
