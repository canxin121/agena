use std::path::PathBuf;

use sea_orm::sqlx::{
    self, Connection, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};

use super::{ffi, register};
use crate::kernel::test_fault::{Control, publish};

struct Fixture {
    _dir: tempfile::TempDir,
    control: PathBuf,
    options: SqliteConnectOptions,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let control = dir.path().join("fault.json");
        publish(&control, &Control::default()).unwrap();
        let name = register(control.clone()).unwrap();
        let options = SqliteConnectOptions::new()
            .filename(dir.path().join("database.sqlite3"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .vfs(name);
        Self {
            _dir: dir,
            control,
            options,
        }
    }

    async fn connect(&self) -> SqliteConnection {
        SqliteConnection::connect_with(&self.options).await.unwrap()
    }

    async fn seed(&self) -> SqliteConnection {
        let mut connection = self.connect().await;
        sqlx::query("CREATE TABLE records (name TEXT PRIMARY KEY)")
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO records VALUES ('durable-before-fault')")
            .execute(&mut connection)
            .await
            .unwrap();
        connection
    }

    fn arm(&self, point: &str, effect: &str) {
        publish(
            &self.control,
            &Control {
                point: point.into(),
                effect: effect.into(),
                remaining: 1,
                arm_id: uuid::Uuid::new_v4().to_string(),
                ..Control::default()
            },
        )
        .unwrap();
    }

    fn state(&self) -> Control {
        serde_json::from_slice(&std::fs::read(&self.control).unwrap()).unwrap()
    }

    fn assert_hit(&self, point: &str, code: i32) {
        let state = self.state();
        assert_eq!(state.point, point);
        assert_eq!(state.sqlite_code, Some(code));
        assert_eq!((state.hits, state.remaining), (1, 0));
        assert!(state.file.as_deref().unwrap().contains("database.sqlite3"));
    }

    async fn assert_recovered(&self) {
        let mut connection = self.connect().await;
        let (integrity,): (String,) = sqlx::query_as("PRAGMA integrity_check")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(integrity, "ok");
        let (count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM records WHERE name='durable-before-fault'")
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(count, 1, "a previously committed row must survive");
        sqlx::query("INSERT INTO records VALUES ('healthy-after-fault')")
            .execute(&mut connection)
            .await
            .unwrap();
        let (count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM records WHERE name='healthy-after-fault'")
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(count, 1);
        connection.close().await.unwrap();
    }
}

fn assert_database_code(error: &sqlx::Error, expected: i32) {
    let code = error
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.into_owned());
    assert_eq!(
        code,
        Some(expected.to_string()),
        "unexpected SQLite error: {error}"
    );
}

#[tokio::test]
async fn disk_full_at_wal_write_rolls_back_transaction_and_recovers() {
    let fixture = Fixture::new();
    let mut connection = fixture.seed().await;
    let mut transaction = connection.begin().await.unwrap();
    sqlx::query("INSERT INTO records VALUES ('not-committed')")
        .execute(&mut *transaction)
        .await
        .unwrap();
    fixture.arm("sqlite.write", "disk_full");
    let error = transaction.commit().await.unwrap_err();
    assert_database_code(&error, ffi::SQLITE_FULL);
    fixture.assert_hit("sqlite.write", ffi::SQLITE_FULL);
    connection.close().await.unwrap();
    let mut reopened = fixture.connect().await;
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM records WHERE name='not-committed'")
            .fetch_one(&mut reopened)
            .await
            .unwrap();
    assert_eq!(
        count, 0,
        "SQLITE_FULL must not publish the interrupted write"
    );
    reopened.close().await.unwrap();
    fixture.assert_recovered().await;
    fixture.assert_hit("sqlite.write", ffi::SQLITE_FULL);
}

#[tokio::test]
async fn write_failure_returns_extended_ioerr_and_preserves_committed_data() {
    let fixture = Fixture::new();
    let mut connection = fixture.seed().await;
    let mut transaction = connection.begin().await.unwrap();
    sqlx::query("INSERT INTO records VALUES ('not-committed')")
        .execute(&mut *transaction)
        .await
        .unwrap();
    fixture.arm("sqlite.write", "io_error");
    let error = transaction.commit().await.unwrap_err();
    assert_database_code(&error, ffi::SQLITE_IOERR_WRITE);
    fixture.assert_hit("sqlite.write", ffi::SQLITE_IOERR_WRITE);
    connection.close().await.unwrap();
    let mut reopened = fixture.connect().await;
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM records WHERE name='not-committed'")
            .fetch_one(&mut reopened)
            .await
            .unwrap();
    assert_eq!(count, 0);
    reopened.close().await.unwrap();
    fixture.assert_recovered().await;
}

#[tokio::test]
async fn sync_failure_is_observed_at_real_commit_and_reopen_recovers() {
    let fixture = Fixture::new();
    let mut connection = fixture.seed().await;
    let mut transaction = connection.begin().await.unwrap();
    sqlx::query("INSERT INTO records VALUES ('commit-outcome-uncertain')")
        .execute(&mut *transaction)
        .await
        .unwrap();
    fixture.arm("sqlite.sync", "io_error");
    let error = transaction.commit().await.unwrap_err();
    assert_database_code(&error, ffi::SQLITE_IOERR_FSYNC);
    fixture.assert_hit("sqlite.sync", ffi::SQLITE_IOERR_FSYNC);
    // A sync error is not a promise that bytes did not reach the filesystem.
    // Recovery must preserve committed data and a structurally valid database.
    connection.close().await.unwrap();
    fixture.assert_recovered().await;
    fixture.assert_hit("sqlite.sync", ffi::SQLITE_IOERR_FSYNC);
}

#[tokio::test]
async fn open_failure_is_one_shot_and_healthy_reopen_succeeds() {
    let fixture = Fixture::new();
    fixture.seed().await.close().await.unwrap();
    fixture.arm("sqlite.open", "io_error");
    let error = SqliteConnection::connect_with(&fixture.options)
        .await
        .unwrap_err();
    assert_database_code(&error, ffi::SQLITE_CANTOPEN);
    fixture.assert_hit("sqlite.open", ffi::SQLITE_CANTOPEN);
    fixture.assert_recovered().await;
    fixture.assert_hit("sqlite.open", ffi::SQLITE_CANTOPEN);
}

#[tokio::test]
async fn read_failure_at_open_returns_extended_ioerr_and_can_retry() {
    let fixture = Fixture::new();
    fixture.seed().await.close().await.unwrap();
    fixture.arm("sqlite.read", "io_error");
    let error = SqliteConnection::connect_with(&fixture.options)
        .await
        .unwrap_err();
    assert_database_code(&error, ffi::SQLITE_IOERR_READ);
    fixture.assert_hit("sqlite.read", ffi::SQLITE_IOERR_READ);
    fixture.assert_recovered().await;
    fixture.assert_hit("sqlite.read", ffi::SQLITE_IOERR_READ);
}

#[tokio::test]
async fn truncate_failure_at_wal_checkpoint_is_one_shot_and_retry_recovers() {
    let fixture = Fixture::new();
    let mut connection = fixture.seed().await;
    fixture.arm("sqlite.truncate", "io_error");
    let error = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_all(&mut connection)
        .await
        .err()
        .expect("truncate must fail");
    assert_database_code(&error, ffi::SQLITE_IOERR_TRUNCATE);
    fixture.assert_hit("sqlite.truncate", ffi::SQLITE_IOERR_TRUNCATE);
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_all(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    fixture.assert_recovered().await;
    fixture.assert_hit("sqlite.truncate", ffi::SQLITE_IOERR_TRUNCATE);
}

#[tokio::test]
async fn named_fault_vfs_does_not_change_default_or_another_cases_vfs() {
    let original = unsafe { ffi::sqlite3_vfs_find(std::ptr::null()) };
    let fixture = Fixture::new();
    let mut connection = fixture.seed().await;
    fixture.arm("sqlite.write", "disk_full");
    let other = Fixture::new();
    other.seed().await.close().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(dir.path().join("default.sqlite3"))
        .create_if_missing(true);
    let mut unaffected = SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::query("CREATE TABLE unaffected (value TEXT)")
        .execute(&mut unaffected)
        .await
        .unwrap();
    sqlx::query("INSERT INTO unaffected VALUES ('normal')")
        .execute(&mut unaffected)
        .await
        .unwrap();
    assert_eq!(fixture.state().hits, 0);
    assert_eq!(unsafe { ffi::sqlite3_vfs_find(std::ptr::null()) }, original);
    let error = sqlx::query("INSERT INTO records VALUES ('interrupted')")
        .execute(&mut connection)
        .await
        .unwrap_err();
    assert_database_code(&error, ffi::SQLITE_FULL);
    fixture.assert_hit("sqlite.write", ffi::SQLITE_FULL);
    connection.close().await.unwrap();
    unaffected.close().await.unwrap();
    fixture.assert_recovered().await;
}
