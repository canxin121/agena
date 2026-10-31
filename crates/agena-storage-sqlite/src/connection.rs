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
        sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteSynchronous},
    },
};

static DATABASE_FILES: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

struct DatabaseOwner {
    _lock: File,
    path: PathBuf,
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
    create_private_parents(parent).map_err(database_error)?;
    let file = path
        .file_name()
        .ok_or_else(|| database_error("database filename is missing"))?;
    Ok(parent.canonicalize().map_err(database_error)?.join(file))
}

fn create_private_parents(path: &Path) -> std::io::Result<()> {
    let mut missing = Vec::new();
    let mut next = path;
    while !next.exists() {
        missing.push(next.to_path_buf());
        let Some(parent) = next
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            break;
        };
        next = parent;
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    // Never change an existing shared ancestor, including /tmp and a caller's
    // workspace. Only newly created application directories become private.
    for directory in missing.into_iter().rev() {
        agena_storage_layout::secure_directory(&directory)?;
    }
    Ok(())
}

fn private_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.nlink() != 1 {
            return Err(std::io::Error::other(
                "hard-linked SQLite files are unsupported",
            ));
        }
    }
    agena_storage_layout::secure_file(path)
}

fn secure_database_files(path: &Path) -> std::io::Result<()> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        match private_file(Path::new(&file)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn acquire_owner(path: &Path, create: bool) -> Result<(PathBuf, Arc<DatabaseOwner>), DbErr> {
    let canonical = canonical_database_path(path)?;
    let mut name = canonical.as_os_str().to_owned();
    name.push(".owner-lock");
    let lock_path = PathBuf::from(name);
    if let Ok(metadata) = std::fs::symlink_metadata(&lock_path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(database_error("SQLite owner lock is not a regular file"));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&lock_path).map_err(database_error)?;
    private_file(&lock_path).map_err(database_error)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(database_error(format!(
                "database {} already has a service owner; connect to that service",
                canonical.display()
            )));
        }
        Err(error) => return Err(database_error(error)),
    };
    if create && !canonical.exists() {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&canonical) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(database_error(error)),
        }
    }
    secure_database_files(&canonical).map_err(database_error)?;
    Ok((
        canonical.clone(),
        Arc::new(DatabaseOwner {
            _lock: file,
            path: canonical,
        }),
    ))
    // Never unlink the lock file: unlinking would let a racing owner lock a
    // different inode while a previous process still holds this one.
}

async fn prepare(
    url: &str,
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
        let create = options
            .to_url_lossy()
            .query_pairs()
            .any(|(key, value)| key == "mode" && value == "rwc");
        let (path, owner) = DATABASE_FILES
            .run(move || acquire_owner(&path, create))
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
        .log_statements(log::LevelFilter::Off);
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
                DATABASE_FILES
                    .run(move || secure_database_files(&owner.path))
                    .await
                    .map_err(|error| sea_orm::sqlx::Error::Io(std::io::Error::other(error)))?
                    .map_err(sea_orm::sqlx::Error::Io)?;
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

/// The owner lock keeps Agena's writers out while these files are copied.
/// Recover only the private copy: even a read-only WAL connection may write
/// shared-memory state, and closing a rejected writer may checkpoint its WAL.
fn copy_recovery_snapshot(path: &Path) -> std::io::Result<Option<tempfile::TempDir>> {
    let mut logs = Vec::new();
    for suffix in ["-wal", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let source = PathBuf::from(name);
        match source.metadata() {
            Ok(metadata) if metadata.len() != 0 => logs.push((source, suffix)),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if logs.is_empty() {
        return Ok(None);
    }
    // Keep the private snapshot on the database filesystem. Clean shutdowns
    // have no recovery log and avoid this copy entirely. Copy failures reject
    // admission rather than falling back to opening the source for recovery.
    let directory = tempfile::Builder::new()
        .prefix(".agena-sqlite-admission-")
        .tempdir_in(path.parent().unwrap_or_else(|| Path::new(".")))?;
    let target = directory.path().join("instance.sqlite");
    std::fs::copy(path, &target)?;
    for (source, suffix) in logs {
        let mut name = target.as_os_str().to_owned();
        name.push(suffix);
        std::fs::copy(source, Path::new(&name))?;
    }
    // SHM is derived coordination state. SQLite rebuilds it in
    // this private directory from the copied WAL rather than sharing it with
    // the source database or trusting a stale WAL index.
    Ok(Some(directory))
}

/// Admit the current baseline without opening any original file for writing.
/// Immutable mode inspects main pages without journal recovery or WAL/SHM
/// creation. Recover retained logs only in a private copy before admitting a
/// writer. Current-baseline schema creation commits before WAL is enabled.
async fn inspect_existing_database(
    options: &SqliteConnectOptions,
    owner: &DatabaseOwner,
) -> Result<(), DbErr> {
    let path = owner.path.clone();
    let has_contents = DATABASE_FILES
        .run(move || -> std::io::Result<bool> {
            if path.metadata()?.len() != 0 {
                return Ok(true);
            }
            // A zero-byte main file with retained SQLite state is not a fresh
            // database. Never initialize over an unresolved journal or WAL.
            for suffix in ["-wal", "-shm", "-journal"] {
                let mut name = path.as_os_str().to_owned();
                name.push(suffix);
                if Path::new(&name).try_exists()? {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .await
        .map_err(database_error)?
        .map_err(database_error)?;
    if !has_contents {
        return Ok(());
    }
    let inspection = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            options
                .clone()
                .read_only(true)
                .create_if_missing(false)
                .immutable(true),
        )
        .await
        .map_err(database_error)?;
    let inspection = SqlxSqliteConnector::from_sqlx_sqlite_pool(inspection);
    let validation = crate::kernel::schema::validate_existing(&inspection).await;
    let closed = inspection.close().await;
    validation?;
    closed?;

    // The main file can still have the current schema while a committed WAL
    // changes a table, index, or baseline identity. Validate the recovered
    // view before a writer can touch any original SQLite file.
    let path = owner.path.clone();
    let snapshot = DATABASE_FILES
        .run(move || copy_recovery_snapshot(&path))
        .await
        .map_err(database_error)?
        .map_err(database_error)?;
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    let inspection = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            options
                .clone()
                .filename(snapshot.path().join("instance.sqlite"))
                .read_only(false)
                .create_if_missing(false)
                .immutable(false),
        )
        .await
        .map_err(database_error)?;
    let inspection = SqlxSqliteConnector::from_sqlx_sqlite_pool(inspection);
    let validation = crate::kernel::schema::validate_existing(&inspection).await;
    let closed = inspection.close().await;
    // Close SQLite before dropping its temporary directory, including on a
    // validation failure. No recovery, checkpoint or cleanup targets source.
    validation?;
    closed
}

/// One writer and up to four readers. Memory mode shares one retained pool
/// rather than opening separate anonymous databases or losing it on idle.
pub struct KernelDatabase {
    pub(crate) writer: Arc<DatabaseConnection>,
    pub(crate) reader: Arc<DatabaseConnection>,
}

impl KernelDatabase {
    pub async fn open(url: &str) -> Result<Self, DbErr> {
        let (options, owner, memory) = prepare(url).await?;
        if let Some(owner) = &owner {
            inspect_existing_database(&options, owner).await?;
        }
        let writer = pool(options.clone(), owner.clone(), 1).await?;
        // Recheck the live view after main-file and recovered-copy admission,
        // before any runtime recovery writes. Fresh files initialize here.
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
mod tests;
