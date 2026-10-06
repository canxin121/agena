//! SeaORM transaction execution for the SQLite infrastructure adapter.
//!
//! Agena databases are shared by multiple processes (one TUI, server, or CLI
//! per process), so write transactions must tolerate concurrent writers.
//! SQLite serializes writers with its single-write lock; the failure mode to
//! avoid is `SQLITE_BUSY` ("database is locked").

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

use agena_async::{WritePermit, WriteQueue};
use agena_storage::TransactionEffects;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, Statement, TransactionTrait,
    Value,
};

#[cfg(test)]
mod failure_tests;

/// Reserved `agena_sequences` row used purely to acquire the SQLite write lock.
///
/// The write-lock fence inserts (or ignores) this row as the first statement of
/// every write transaction. It is never read by the sequence allocators, which
/// only touch their own named rows (`seq_global`, `message_id`, `part_id`, and
/// per-session rows).
const WRITE_LOCK_SEQUENCE: &str = "__agena_write_lock__";

/// Number of times a write transaction retries a busy lock before giving up.
const MAX_BUSY_RETRIES: usize = 5;
const WRITE_LOCK_DEADLINE: Duration = Duration::from_secs(10);
const ADMISSION_ERROR_PREFIX: &str = "SQLite write admission failed: ";

/// Admission overload is transient, but has no SQLite driver error code.
/// Pool checkout timeouts likewise indicate exhausted database capacity.
pub fn is_write_admission_busy(error: &DbErr) -> bool {
    matches!(error, DbErr::Custom(message) if message.starts_with(ADMISSION_ERROR_PREFIX))
        || matches!(
            error,
            DbErr::ConnectionAcquire(sea_orm::ConnAcquireErr::Timeout)
        )
}

/// Pool clones and all repositories on the same file share one writer queue.
/// Cache by the pool's retained connect-options Arc, so pointer reuse cannot
/// accidentally associate a newly opened database with an old pool's queue.
pub async fn acquire_write_permit(db: &DatabaseConnection) -> Result<WritePermit, DbErr> {
    type Options = sea_orm::sqlx::sqlite::SqliteConnectOptions;
    type QueueCell = tokio::sync::OnceCell<Arc<WriteQueue>>;
    type Registry = HashMap<usize, (Weak<Options>, Arc<QueueCell>)>;
    static POOLS: OnceLock<Mutex<Registry>> = OnceLock::new();
    let options = db.get_sqlite_connection_pool().connect_options();
    let cell = {
        let mut pools = POOLS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pools.retain(|_, (options, _)| options.strong_count() > 0);
        Arc::clone(
            &pools
                .entry(Arc::as_ptr(&options) as usize)
                .or_insert_with(|| (Arc::downgrade(&options), Arc::new(QueueCell::new())))
                .1,
        )
    };
    let queue = cell
        .get_or_try_init(|| async {
            let row = db
                .query_one(Statement::from_string(
                    db.get_database_backend(),
                    "PRAGMA database_list",
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("SQLite main database is missing".into()))?;
            let file: String = row.try_get("", "file")?;
            if file.is_empty() {
                // SQLx gives each anonymous memory database a unique filename.
                // Named shared-cache memory pools deliberately share admission.
                Ok(WriteQueue::named(
                    format!("sqlite-memory:{}", options.get_filename().display()).into(),
                ))
            } else {
                WriteQueue::for_file(std::path::Path::new(&file))
                    .await
                    .map_err(|error| DbErr::Custom(format!("resolve SQLite write queue: {error}")))
            }
        })
        .await?;
    queue
        .acquire()
        .await
        .map_err(|error| DbErr::Custom(format!("{ADMISSION_ERROR_PREFIX}{error}")))
}

/// Acquire the SQLite write lock as the first statement of a transaction.
///
/// SeaORM begins every transaction with `BEGIN` (SQLite `BEGIN DEFERRED`). A
/// deferred transaction that runs a `SELECT` before its first write must
/// upgrade from a read lock to the write lock mid-transaction, and SQLite
/// can return `SQLITE_BUSY` immediately for that upgrade (or `BUSY_SNAPSHOT`
/// in WAL mode) without a useful busy-timeout wait. Issuing a benign write
/// as the first statement moves the lock acquisition to the point where the
/// busy timeout applies, so concurrent writers wait instead of failing.
///
/// The write is `INSERT OR IGNORE` on a reserved `agena_sequences` row: the
/// first call inserts the sentinel, every later call is a no-op, and it still
/// acquires the write lock every time (SQLite takes the write lock before it
/// evaluates whether the statement changes anything).
pub async fn acquire_write_lock(transaction: &DatabaseTransaction) -> Result<(), DbErr> {
    transaction
        .execute(Statement::from_sql_and_values(
            transaction.get_database_backend(),
            "INSERT OR IGNORE INTO agena_sequences (seq_name, next_val) VALUES (?, 1)",
            [Value::from(WRITE_LOCK_SEQUENCE)],
        ))
        .await
        .map(|_| ())
}

/// Whether a `DbErr` is a transient SQLite lock conflict.
///
/// `SQLITE_BUSY` (5) and its extended codes, including `BUSY_RECOVERY` (261),
/// `BUSY_SNAPSHOT` (517), and `BUSY_TIMEOUT` (773), share primary code 5 in
/// the low byte. A snapshot conflict requires restarting the transaction;
/// waiting or retrying a statement in the same snapshot cannot resolve it.
/// Structured codes are authoritative. Message fallback is used only when the
/// SQLx database error does not expose a numeric code.
pub fn is_sqlite_busy(error: &DbErr) -> bool {
    let sqlx_error = match error {
        DbErr::Exec(sea_orm::RuntimeErr::SqlxError(error))
        | DbErr::Query(sea_orm::RuntimeErr::SqlxError(error))
        | DbErr::Conn(sea_orm::RuntimeErr::SqlxError(error)) => error,
        _ => return false,
    };
    match sqlx_error {
        sea_orm::sqlx::Error::Database(error) => {
            if let Some(code) = error.code().and_then(|code| code.parse::<u32>().ok()) {
                return code & 0xff == 5;
            }
            error
                .message()
                .to_ascii_lowercase()
                .contains("database is locked")
        }
        _ => false,
    }
}

/// Exponential backoff between busy-lock retries.
fn busy_backoff(attempt: usize) -> std::time::Duration {
    // 100ms, 200ms, 400ms, 800ms, 1.6s — capped so a fifth retry is not a
    // pathological sleep on top of the per-connection busy timeout.
    std::time::Duration::from_millis(100 << attempt.min(4))
}

/// Runs a SQLite/SeaORM transaction and executes queued effects only after commit.
///
/// The write lock is acquired before any user statement runs (see
/// [`acquire_write_lock`]) so the per-connection busy timeout applies to the
/// lock wait instead of surfacing `SQLITE_BUSY` on a mid-transaction upgrade.
/// If the lock cannot be acquired, the transaction is retried with backoff; the
/// caller closure is invoked exactly once, on the successful attempt.
pub async fn run_transaction_effects<T, O>(db: &DatabaseConnection, op: O) -> Result<T, DbErr>
where
    O: for<'a> FnOnce(
        &'a DatabaseTransaction,
        &'a mut TransactionEffects,
    ) -> Pin<Box<dyn Future<Output = Result<T, DbErr>> + Send + 'a>>,
{
    let (transaction, permit) = begin_with_write_lock(db).await?;
    let mut effects = TransactionEffects::new();
    match op(&transaction, &mut effects).await {
        Ok(value) => {
            transaction.commit().await?;
            drop(permit);
            effects.run().await;
            Ok(value)
        }
        Err(error) => {
            rollback_after_operation_error(transaction).await;
            Err(error)
        }
    }
}

/// Error-generic variant for application-facing transaction choreography.
pub async fn run_transaction_app_effects<T, E, O>(db: &DatabaseConnection, op: O) -> Result<T, E>
where
    E: From<DbErr>,
    O: for<'a> FnOnce(
        &'a DatabaseTransaction,
        &'a mut TransactionEffects,
    ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>,
{
    let (transaction, permit) = begin_with_write_lock(db).await.map_err(E::from)?;
    let mut effects = TransactionEffects::new();
    match op(&transaction, &mut effects).await {
        Ok(value) => {
            transaction.commit().await.map_err(E::from)?;
            drop(permit);
            effects.run().await;
            Ok(value)
        }
        Err(error) => {
            rollback_after_operation_error(transaction).await;
            Err(error)
        }
    }
}

async fn rollback_after_operation_error(transaction: DatabaseTransaction) {
    // SQLite can already have rolled back the transaction (e.g. RAISE(ROLLBACK)
    // or an I/O failure). Keep the operation's original error and classification,
    // including application-specific variants, even if explicit cleanup fails.
    if let Err(error) = transaction.rollback().await {
        tracing::error!(
            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                "roll back a failed SQLite transaction",
                &error,
            ),
            "SQLite transaction rollback failed; preserving the operation error"
        );
    }
}

/// Begin a transaction and acquire the write lock immediately, retrying a busy
/// lock with backoff. Returns a transaction that already holds the write lock.
pub(crate) async fn begin_with_write_lock(
    db: &DatabaseConnection,
) -> Result<(DatabaseTransaction, WritePermit), DbErr> {
    let permit = acquire_write_permit(db).await?;
    let started = Instant::now();
    let mut attempt = 0usize;
    loop {
        let transaction = db.begin().await?;
        match acquire_write_lock(&transaction).await {
            Ok(()) => return Ok((transaction, permit)),
            Err(error) => {
                if let Err(rollback_error) = transaction.rollback().await {
                    return Err(DbErr::Custom(format!(
                        "{}; additionally, {}",
                        agena_failure::diagnostic::format_error_chain(&error),
                        agena_failure::diagnostic::format_error_chain_with_context(
                            "failed to roll back after acquiring the SQLite write lock failed",
                            &rollback_error,
                        )
                    )));
                }
                if is_sqlite_busy(&error)
                    && attempt < MAX_BUSY_RETRIES
                    && started.elapsed() < WRITE_LOCK_DEADLINE
                {
                    // Rollback returned the connection to the pool before the
                    // backoff. No user statement/effect is retried here.
                    let remaining = WRITE_LOCK_DEADLINE.saturating_sub(started.elapsed());
                    tokio::time::sleep(busy_backoff(attempt).min(remaining)).await;
                    attempt += 1;
                } else {
                    return Err(error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use sea_orm::{ConnectionTrait, Database};

    use super::{DbErr, is_sqlite_busy, run_transaction_effects};

    /// Open an in-memory database with the full schema, as every production
    /// connection gets via `initialize_schema` before any repository runs.
    async fn initialized_database() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("open database");
        crate::initialize_schema(&db)
            .await
            .expect("initialize schema");
        db
    }

    #[tokio::test]
    async fn effects_run_only_after_a_successful_commit() {
        let db = initialized_database().await;
        let committed = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&committed);
        run_transaction_effects(&db, move |_transaction, effects| {
            effects.push(async move { flag.store(true, Ordering::SeqCst) });
            Box::pin(async { Ok(()) })
        })
        .await
        .expect("commit transaction");
        assert!(committed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn effects_do_not_run_after_a_rollback() {
        let db = initialized_database().await;
        let committed = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&committed);
        let result: Result<(), DbErr> =
            run_transaction_effects(&db, move |_transaction, effects| {
                effects.push(async move { flag.store(true, Ordering::SeqCst) });
                Box::pin(async { Err(DbErr::Custom("abort transaction".to_owned())) })
            })
            .await;
        assert!(result.is_err());
        assert!(!committed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn write_lock_fence_is_idempotent_across_transactions() {
        let db = initialized_database().await;
        // First transaction inserts the sentinel row; later ones ignore it.
        for _ in 0..3 {
            run_transaction_effects(&db, |_transaction, _effects| Box::pin(async { Ok(()) }))
                .await
                .expect("transaction with write-lock fence");
        }
        let count: i64 = db
            .query_one(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                format!(
                    "SELECT COUNT(*) AS count FROM agena_sequences WHERE seq_name = '{}'",
                    super::WRITE_LOCK_SEQUENCE
                ),
            ))
            .await
            .expect("count sentinel")
            .expect("count row")
            .try_get("", "count")
            .expect("count value");
        assert_eq!(count, 1, "write-lock sentinel row is inserted exactly once");
    }

    #[tokio::test]
    async fn busy_detection_recognizes_sqlite_busy_and_rejects_others() {
        // SQLITE_BUSY via structured code (5) and SQLITE_BUSY_SNAPSHOT (517).
        assert!(is_sqlite_busy(&db_error_with("5", "database is locked")));
        assert!(is_sqlite_busy(&db_error_with(
            "517",
            "database table is locked"
        )));
        // An unrelated SQLite code and a custom error are not busy.
        assert!(!is_sqlite_busy(&db_error_with(
            "1",
            "no such table: missing"
        )));
        assert!(!is_sqlite_busy(&sea_orm::DbErr::Custom(
            "unrelated".to_owned()
        )));
        // A wrapper that never contains a SQLx database error is not busy.
        assert!(!is_sqlite_busy(&sea_orm::DbErr::RecordNotFound(
            "x".to_owned()
        )));
    }

    /// Builds a `DbErr::Exec` carrying a fake `DatabaseError` with the given
    /// SQLite code and message. `SqliteError`'s constructors are private, so a
    /// test double over the public `DatabaseError` trait stands in for it.
    pub(super) fn db_error_with(code: &'static str, message: &'static str) -> sea_orm::DbErr {
        sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(
            sea_orm::sqlx::Error::Database(Box::new(FakeDatabaseError { message, code })),
        ))
    }

    #[derive(Debug)]
    struct FakeDatabaseError {
        message: &'static str,
        code: &'static str,
    }

    impl std::fmt::Display for FakeDatabaseError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "(code: {}) {}", self.code, self.message)
        }
    }

    impl std::error::Error for FakeDatabaseError {}

    impl sea_orm::sqlx::error::DatabaseError for FakeDatabaseError {
        fn message(&self) -> &str {
            self.message
        }
        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some(self.code.into())
        }
        fn kind(&self) -> sea_orm::sqlx::error::ErrorKind {
            sea_orm::sqlx::error::ErrorKind::Other
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
    }
}
