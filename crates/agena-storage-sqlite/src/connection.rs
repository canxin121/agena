//! SQLite lifecycle ownership and per-connection resource/durability policy.
//!
//! File locks live in the SQLx pool's retained connection callback. Every pool
//! clone therefore retains ownership; there is no global lock registry, stale
//! PID file, or task whose cancellation can silently release the owner. Memory
//! pools keep their sole private connection without checkout hooks or rotation.

use std::{
    fs::File,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use sea_orm::{
    DatabaseConnection, DbErr, SqlxSqliteConnector,
    sqlx::{
        ConnectOptions,
        sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    },
};

static DATABASE_FILES: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

struct DatabaseOwner {
    _lock: File,
}

fn database_error(error: impl std::fmt::Display) -> DbErr {
    DbErr::Custom(format!("SQLite lifecycle: {error}"))
}

fn memory_url(url: &str) -> bool {
    let raw = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    let (database, query) = raw.split_once('?').unwrap_or((raw, ""));
    database == ":memory:"
        || url::form_urlencoded::parse(query.as_bytes())
            .any(|(key, value)| key == "mode" && value == "memory")
}

pub fn is_memory_database(url: &str) -> bool {
    url.starts_with("sqlite:") && SqliteConnectOptions::from_str(url).is_ok_and(|_| memory_url(url))
}

/// Canonicalize parent symlinks before creating a new database. For existing
/// files canonicalize the file too, so aliases acquire the same lock.
fn canonical_database_path(path: &Path) -> Result<PathBuf, DbErr> {
    if path.exists() {
        let path = path.canonicalize().map_err(database_error)?;
        let metadata = path.metadata().map_err(database_error)?;
        if !metadata.is_file() {
            return Err(database_error("database is not a regular file"));
        }
        // SQLite WAL itself cannot safely use multiple hard-link names. Refuse
        // that configuration instead of pretending a path lock covers aliases.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() > 1 {
                return Err(database_error(
                    "hard-linked databases are unsupported; use one canonical database path",
                ));
            }
        }
        return Ok(path);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(database_error)?;
    let file = path
        .file_name()
        .ok_or_else(|| database_error("database filename is missing"))?;
    Ok(parent.canonicalize().map_err(database_error)?.join(file))
}

fn acquire_owner(path: &Path) -> Result<(PathBuf, Arc<DatabaseOwner>), DbErr> {
    let canonical = canonical_database_path(path)?;
    let mut name = canonical.as_os_str().to_owned();
    name.push(".owner-lock");
    let lock_path = PathBuf::from(name);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(database_error)?;
    match file.try_lock() {
        Ok(()) => Ok((canonical, Arc::new(DatabaseOwner { _lock: file }))),
        Err(std::fs::TryLockError::WouldBlock) => Err(database_error(format!(
            "database {} already has a service owner; connect to that service",
            canonical.display()
        ))),
        Err(error) => Err(database_error(error)),
    }
    // Never unlink the lock file: unlinking would let a racing owner lock a
    // different inode while a previous process still holds this one.
}

async fn prepare(
    url: &str,
    level: log::LevelFilter,
) -> Result<(SqliteConnectOptions, Option<Arc<DatabaseOwner>>, bool), DbErr> {
    if !url.starts_with("sqlite:") {
        return Err(database_error("only SQLite database URLs are supported"));
    }
    let mut options = SqliteConnectOptions::from_str(url).map_err(database_error)?;
    let memory = memory_url(url);
    let owner = if memory {
        None
    } else {
        let path = options.get_filename().to_owned();
        let (path, owner) = DATABASE_FILES
            .run(move || acquire_owner(&path))
            .await
            .map_err(database_error)??;
        options = options.filename(path);
        Some(owner)
    };
    options = options
        .foreign_keys(true)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(Duration::from_secs(1))
        .pragma("cache_size", "-8192")
        .pragma("mmap_size", "0")
        .pragma("wal_autocheckpoint", "1000")
        .log_statements(level);
    if memory {
        // Named memory URIs must not accidentally give two independent
        // factories competing authority over the same anonymous instance.
        // This factory retains exactly one private connection.
        options = options.shared_cache(false);
    }
    Ok((options, owner, memory))
}

async fn pool(
    options: SqliteConnectOptions,
    owner: Option<Arc<DatabaseOwner>>,
    max: u32,
) -> Result<Arc<DatabaseConnection>, DbErr> {
    let pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(max)
        .acquire_timeout(Duration::from_secs(3));
    let pool = match owner {
        Some(owner) => pool.after_connect(move |_, _| {
            let owner = owner.clone();
            Box::pin(async move {
                drop(owner);
                Ok(())
            })
        }),
        // Only private memory databases have no file owner. SQLx can discard
        // a connection when acquire is cancelled during a checkout ping or
        // hook; replacing our sole memory connection would erase the database.
        // Follow Pool::acquire's cancellation contract: no checkout ping,
        // before_acquire or after_connect hooks, and no connection rotation.
        None => pool
            .test_before_acquire(false)
            .idle_timeout(None)
            .max_lifetime(None),
    };
    let pool = pool.connect_with(options).await.map_err(database_error)?;
    Ok(Arc::new(SqlxSqliteConnector::from_sqlx_sqlite_pool(pool)))
}

/// Infrastructure repositories, including scheduler/server state, use one
/// owned pool. The session kernel uses `KernelDatabase`'s dedicated writer.
pub async fn connect_owned_database(
    url: &str,
    level: log::LevelFilter,
) -> Result<DatabaseConnection, DbErr> {
    let (mut options, owner, memory) = prepare(url, level).await?;
    if !memory {
        options = options.journal_mode(SqliteJournalMode::Wal);
    }
    let connection = pool(options, owner, if memory { 1 } else { 5 }).await?;
    Arc::try_unwrap(connection).map_err(|_| database_error("new pool unexpectedly shared"))
}

/// One writer and up to four readers. Memory mode shares one retained pool
/// rather than opening separate anonymous databases or losing it on idle.
pub struct KernelDatabase {
    pub(crate) writer: Arc<DatabaseConnection>,
    pub(crate) reader: Arc<DatabaseConnection>,
}

impl KernelDatabase {
    pub async fn open(url: &str) -> Result<Self, DbErr> {
        let (options, owner, memory) = prepare(url, log::LevelFilter::Off).await?;
        let writer = pool(options.clone(), owner.clone(), 1).await?;
        // Inspect an existing schema before switching journal mode or making
        // recovery writes. Rejecting an old database must preserve its bytes.
        if let Err(error) = crate::kernel::schema::initialize(writer.as_ref()).await {
            let _ = writer.close_by_ref().await;
            return Err(error);
        }
        if !memory {
            use sea_orm::ConnectionTrait;
            writer
                .execute(sea_orm::Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    "PRAGMA journal_mode=WAL",
                ))
                .await?;
        }
        let reader = if memory {
            writer.clone()
        } else {
            pool(options.pragma("query_only", "ON"), owner, 4).await?
        };
        Ok(Self { writer, reader })
    }

    /// Wait for statements to drain. Ownership remains held until every pool
    /// clone is dropped, including clones retained by background work.
    pub async fn close(self) -> Result<(), DbErr> {
        self.reader.close_by_ref().await?;
        self.writer.close_by_ref().await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };

    use super::*;
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

    #[tokio::test]
    async fn cancelled_memory_pool_acquires_preserve_schema_and_committed_data() {
        let database = KernelDatabase::open("sqlite::memory:").await.unwrap();
        let pool = database.writer.get_sqlite_connection_pool();
        let mut transaction = pool.begin().await.unwrap();
        sea_orm::sqlx::query("CREATE TABLE cancellation_marker (value TEXT NOT NULL)")
            .execute(&mut *transaction)
            .await
            .unwrap();
        sea_orm::sqlx::query(
            "INSERT INTO cancellation_marker VALUES ('committed before cancellation')",
        )
        .execute(&mut *transaction)
        .await
        .unwrap();
        transaction.commit().await.unwrap();

        for attempt in 0..64 {
            tokio::time::timeout(Duration::from_secs(3), async {
                while pool.num_idle() != 1 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("the sole memory connection must return to the idle pool");

            // Poll the real idle checkout once, then cancel it at its first
            // suspension point. A checkout ping used to suspend after removing
            // the sole connection from the pool, so dropping acquire destroyed
            // both the schema and committed data. With no checkout hooks the
            // connection can instead be returned immediately and dropped safely.
            let checkout = {
                let mut acquire = Box::pin(pool.acquire());
                poll_fn(|cx| Poll::Ready(acquire.as_mut().poll(cx))).await
            };
            if let Poll::Ready(result) = checkout {
                drop(result.expect("idle memory checkout must succeed"));
            }

            // Also cancel a guaranteed pending checkout while another caller
            // owns the connection, exercising the semaphore wait/handoff path.
            let held = pool.acquire().await.unwrap();
            {
                let mut acquire = Box::pin(pool.acquire());
                assert!(
                    poll_fn(|cx| Poll::Ready(acquire.as_mut().poll(cx)))
                        .await
                        .is_pending(),
                    "the singleton memory pool must queue a second checkout"
                );
            }
            drop(held);

            let marker: String =
                sea_orm::sqlx::query_scalar("SELECT value FROM cancellation_marker")
                    .fetch_one(pool)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("cancelled acquire {attempt} lost committed memory data: {error}")
                    });
            assert_eq!(marker, "committed before cancellation");
            let snapshots: i64 =
                sea_orm::sqlx::query_scalar("SELECT COUNT(*) FROM agena_history_snapshots")
                    .fetch_one(pool)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("cancelled acquire {attempt} lost the kernel schema: {error}")
                    });
            assert_eq!(snapshots, 0);
        }

        database.close().await.unwrap();
    }

    #[tokio::test]
    async fn rejecting_an_old_database_preserves_its_file_bytes_and_journal_mode() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("old.db");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Delete);
        let fixture = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sea_orm::sqlx::query(
            "CREATE TABLE obsolete_fixture(id INTEGER PRIMARY KEY, body TEXT NOT NULL)",
        )
        .execute(&fixture)
        .await
        .unwrap();
        sea_orm::sqlx::query(
            "INSERT INTO obsolete_fixture VALUES (1,'user database remains intact')",
        )
        .execute(&fixture)
        .await
        .unwrap();
        fixture.close().await;
        let before = std::fs::read(&path).unwrap();
        let error = KernelDatabase::open(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("fresh database path"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("db-wal").exists());
    }

    #[tokio::test]
    async fn pools_share_memory_and_every_file_connection_has_full_policy() {
        for memory in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let url = if memory {
                "sqlite::memory:".to_owned()
            } else {
                format!(
                    "sqlite://{}?mode=rwc",
                    dir.path().join("state.db").display()
                )
            };
            let database = KernelDatabase::open(&url).await.unwrap();
            database
                .writer
                .execute(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "CREATE TABLE shared (id INTEGER PRIMARY KEY)",
                ))
                .await
                .unwrap();
            database
                .writer
                .execute(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    "INSERT INTO shared VALUES (1)",
                ))
                .await
                .unwrap();
            assert!(
                database
                    .reader
                    .query_one(Statement::from_string(
                        DatabaseBackend::Sqlite,
                        "SELECT id FROM shared"
                    ))
                    .await
                    .unwrap()
                    .is_some()
            );
            let reader_pool = database.reader.get_sqlite_connection_pool();
            let mut retained = Vec::new();
            for _ in 0..if memory { 1 } else { 4 } {
                let mut connection = reader_pool.acquire().await.unwrap();
                for (pragma, expected) in [
                    ("synchronous", 2i64),
                    ("foreign_keys", 1),
                    ("cache_size", -8192),
                    ("mmap_size", 0),
                ] {
                    if memory && pragma == "mmap_size" {
                        continue;
                    }
                    let value: i64 = sea_orm::sqlx::query_scalar(&format!("PRAGMA {pragma}"))
                        .fetch_one(&mut *connection)
                        .await
                        .unwrap();
                    assert_eq!(value, expected, "{pragma}");
                }
                retained.push(connection);
            }
            drop(retained);
            if !memory {
                assert!(
                    database
                        .reader
                        .execute(Statement::from_string(
                            DatabaseBackend::Sqlite,
                            "INSERT INTO shared VALUES (2)"
                        ))
                        .await
                        .is_err()
                );
                assert!(KernelDatabase::open(&url).await.is_err());
            }
            database.close().await.unwrap();
        }
        assert!(is_memory_database("sqlite://named?mode=memory"));
        assert!(!is_memory_database(
            "sqlite://file:memory:backup.db?mode=rwc"
        ));
    }
}
