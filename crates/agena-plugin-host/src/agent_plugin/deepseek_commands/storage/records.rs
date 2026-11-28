//! Durable Cordis rows stored in an instance-private SQLite database.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use agena_plugin_sdk::{HostClient, PluginError, Result};
use sea_orm::sqlx::{
    self, Row, SqliteConnection, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::OnceCell;

use super::valid_unit;

const MAX_UNITS: i64 = 64;
const MAX_ROWS: i64 = 4096;
const MAX_VALUE_BYTES: usize = 128 * 1024;
const MAX_SNAPSHOT_BYTES: i64 = 2 * 1024 * 1024;
const MAX_BACKUP_BYTES: i64 = 8 * 1024 * 1024;
const MAX_VERSION: i64 = (1_i64 << 53) - 1;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Backend {
    JsonRecords,
    Sqlite,
}

impl Backend {
    fn file_prefix(self) -> &'static str {
        match self {
            Self::JsonRecords => "json-records",
            Self::Sqlite => "sqlite",
        }
    }
}

pub(super) struct RecordStore {
    backend: Backend,
    namespace: String,
    memory: bool,
    journal: SqliteJournalMode,
    opened: OnceCell<OpenedStore>,
}

struct OpenedStore {
    directory: PathBuf,
    database: Option<PathBuf>,
    pool: SqlitePool,
}

#[derive(Serialize)]
pub(super) struct Unit {
    revision: i64,
    version: i64,
    entries: Vec<Entry>,
}

#[derive(Serialize)]
struct Entry {
    table: String,
    key: String,
    version: i64,
    value: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::agent_plugin::deepseek_commands) struct Mutation {
    pub(super) backend: Backend,
    unit: String,
    expected: Option<i64>,
    version: i64,
    edits: Vec<Edit>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Edit {
    Put {
        table: String,
        key: String,
        value: Value,
    },
    Delete {
        table: String,
        key: String,
    },
    Backup {
        table: String,
        key: String,
    },
}

#[derive(Serialize)]
pub(super) struct Committed {
    revision: i64,
    backups: Vec<String>,
}

impl RecordStore {
    pub(super) fn new(
        backend: Backend,
        namespace: String,
        memory: bool,
        journal: SqliteJournalMode,
    ) -> Self {
        Self {
            backend,
            namespace,
            memory,
            journal,
            opened: OnceCell::new(),
        }
    }

    async fn opened(&self, host: &dyn HostClient, create: bool) -> Result<Option<&OpenedStore>> {
        let directory = host.imported_plugin_storage_directory().await?;
        let path = (!self.memory).then(|| {
            directory.join("cordis").join(format!(
                "{}.{}.sqlite3",
                self.backend.file_prefix(),
                self.namespace
            ))
        });
        if !create && self.opened.get().is_none() {
            match &path {
                Some(path)
                    if tokio::fs::try_exists(path)
                        .await
                        .map_err(|error| PluginError::internal_error(&error))? => {}
                _ => return Ok(None),
            }
        }
        let opened = self
            .opened
            .get_or_try_init(|| async {
                let mut options = SqliteConnectOptions::new()
                    .busy_timeout(Duration::from_secs(5))
                    .foreign_keys(true)
                    .synchronous(SqliteSynchronous::Full);
                if let Some(path) = &path {
                    let file = path.clone();
                    tokio::task::spawn_blocking(move || prepare_database_file(file))
                        .await
                        .map_err(|error| PluginError::internal_error(&error))?
                        .map_err(|error| PluginError::internal_error(&error))?;
                    options = options.filename(&path).journal_mode(self.journal);
                } else {
                    options = options.filename(":memory:").in_memory(true);
                }
                let pool = SqlitePoolOptions::new()
                    .min_connections(1)
                    .max_connections(1)
                    .test_before_acquire(false)
                    .idle_timeout(None)
                    .max_lifetime(None)
                    .connect_with(options)
                    .await
                    .map_err(sql_error)?;
                sqlx::raw_sql(
                    "CREATE TABLE IF NOT EXISTS cordis_units (
                    name TEXT PRIMARY KEY, revision INTEGER NOT NULL, version INTEGER NOT NULL
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS cordis_records (
                    unit TEXT NOT NULL REFERENCES cordis_units(name),
                    table_name TEXT NOT NULL, record_key TEXT NOT NULL,
                    version INTEGER NOT NULL, value TEXT NOT NULL,
                    PRIMARY KEY (unit, table_name, record_key)
                 ) STRICT;
                 CREATE TABLE IF NOT EXISTS cordis_backups (
                    id INTEGER PRIMARY KEY, unit TEXT NOT NULL, table_name TEXT NOT NULL,
                    record_key TEXT NOT NULL, version INTEGER NOT NULL, value TEXT NOT NULL
                 ) STRICT;",
                )
                .execute(&pool)
                .await
                .map_err(sql_error)?;
                Ok::<_, PluginError>(OpenedStore {
                    directory: directory.clone(),
                    database: path,
                    pool,
                })
            })
            .await?;
        if opened.directory != directory {
            return Err(PluginError::internal(
                "Cordis storage directory changed between plugin calls",
            ));
        }
        Ok(Some(opened))
    }

    pub(super) async fn snapshot(&self, host: &dyn HostClient) -> Result<BTreeMap<String, Unit>> {
        let Some(opened) = self.opened(host, false).await? else {
            return Ok(BTreeMap::new());
        };
        let mut transaction = opened.pool.begin().await.map_err(sql_error)?;
        check_capacity(&mut transaction).await?;
        let mut units = BTreeMap::new();
        for row in sqlx::query("SELECT name, revision, version FROM cordis_units")
            .fetch_all(&mut *transaction)
            .await
            .map_err(sql_error)?
        {
            let name: String = row.try_get("name").map_err(sql_error)?;
            let revision = row.try_get("revision").map_err(sql_error)?;
            let version = row.try_get("version").map_err(sql_error)?;
            if !valid_unit(&name)
                || !(1..=MAX_VERSION).contains(&revision)
                || !(0..=MAX_VERSION).contains(&version)
            {
                return Err(malformed());
            }
            units.insert(
                name,
                Unit {
                    revision,
                    version,
                    entries: Vec::new(),
                },
            );
        }
        for row in sqlx::query(
            "SELECT unit, table_name, record_key, version, value FROM cordis_records
             ORDER BY unit, table_name, record_key",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(sql_error)?
        {
            let name: String = row.try_get("unit").map_err(sql_error)?;
            let table = row.try_get("table_name").map_err(sql_error)?;
            let key = row.try_get("record_key").map_err(sql_error)?;
            let version = row.try_get("version").map_err(sql_error)?;
            let raw: String = row.try_get("value").map_err(sql_error)?;
            let value = match serde_json::from_str(&raw) {
                Ok(value) => value,
                Err(_) if matches!(self.backend, Backend::JsonRecords) => continue,
                Err(_) => return Err(malformed()),
            };
            units
                .get_mut(&name)
                .ok_or_else(malformed)?
                .entries
                .push(Entry {
                    table,
                    key,
                    version,
                    value,
                });
        }
        transaction.commit().await.map_err(sql_error)?;
        Ok(units)
    }

    pub(super) async fn commit(
        &self,
        host: &dyn HostClient,
        mutation: Mutation,
    ) -> Result<Option<Committed>> {
        if !valid_unit(&mutation.unit)
            || !(0..=MAX_VERSION).contains(&mutation.version)
            || mutation.edits.is_empty()
            || mutation.edits.len() > MAX_ROWS as usize
        {
            return Err(PluginError::invalid_params(
                "invalid Cordis record transaction",
            ));
        }
        let opened = self.opened(host, true).await?.ok_or_else(malformed)?;
        let mut transaction = opened
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sql_error)?;
        let current =
            sqlx::query_scalar::<_, i64>("SELECT revision FROM cordis_units WHERE name = ?")
                .bind(&mutation.unit)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(sql_error)?;
        if current != mutation.expected {
            return Ok(None);
        }
        let revision = current
            .unwrap_or_default()
            .checked_add(1)
            .filter(|revision| *revision <= MAX_VERSION)
            .ok_or_else(|| PluginError::internal("Cordis storage revision exhausted"))?;
        sqlx::query(
            "INSERT INTO cordis_units (name, revision, version) VALUES (?, ?, ?)
             ON CONFLICT(name) DO UPDATE SET revision=excluded.revision, version=excluded.version",
        )
        .bind(&mutation.unit)
        .bind(revision)
        .bind(mutation.version)
        .execute(&mut *transaction)
        .await
        .map_err(sql_error)?;
        let mut backups = Vec::new();
        for edit in mutation.edits {
            let id = apply_edit(
                &mut transaction,
                self.backend,
                &mutation.unit,
                mutation.version,
                edit,
            )
            .await?;
            if let Some(id) = id {
                let path = opened.database.as_ref().ok_or_else(malformed)?;
                backups.push(format!("{}#cordis_backups/{id}", path.display()));
            }
        }
        check_capacity(&mut transaction).await?;
        if !backups.is_empty() {
            let (count, bytes) = sqlx::query_as::<_, (i64, i64)>(
                "SELECT count(*), coalesce(sum(length(CAST(value AS BLOB))), 0) FROM cordis_backups",
            ).fetch_one(&mut *transaction).await.map_err(sql_error)?;
            if count > MAX_ROWS || bytes > MAX_BACKUP_BYTES {
                return Err(PluginError::invalid_params(
                    "Cordis record backups exceed their capacity",
                ));
            }
        }
        transaction.commit().await.map_err(sql_error)?;
        Ok(Some(Committed { revision, backups }))
    }
}

async fn apply_edit(
    connection: &mut SqliteConnection,
    backend: Backend,
    unit: &str,
    version: i64,
    edit: Edit,
) -> Result<Option<i64>> {
    let (table, key) = match &edit {
        Edit::Put { table, key, .. }
        | Edit::Delete { table, key }
        | Edit::Backup { table, key } => (table, key),
    };
    let global = table.is_empty() && key.is_empty() && matches!(edit, Edit::Put { .. });
    let path_safe = |key: &str| {
        !key.is_empty()
            && key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    };
    if !global
        && (!valid_unit(table)
            || key.len() > 4096
            || matches!(backend, Backend::JsonRecords) && !path_safe(key))
    {
        return Err(PluginError::invalid_params(
            "invalid Cordis record identity",
        ));
    }
    if let Edit::Put { value, .. } = &edit {
        let value =
            serde_json::to_string(value).map_err(|error| PluginError::internal_error(&error))?;
        if value.len() > MAX_VALUE_BYTES {
            return Err(PluginError::invalid_params(
                "Cordis record exceeds its byte budget",
            ));
        }
        sqlx::query(
            "INSERT INTO cordis_records (unit, table_name, record_key, version, value) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(unit, table_name, record_key) DO UPDATE SET version=excluded.version, value=excluded.value",
        ).bind(unit).bind(table).bind(key).bind(version).bind(value)
            .execute(connection).await.map_err(sql_error)?;
        return Ok(None);
    }
    let id = if matches!(edit, Edit::Backup { .. }) {
        if !matches!(backend, Backend::JsonRecords) {
            return Err(PluginError::invalid_params(
                "this Cordis backend cannot back up a record",
            ));
        }
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO cordis_backups (unit, table_name, record_key, version, value)
             SELECT unit, table_name, record_key, version, value FROM cordis_records
             WHERE unit = ? AND table_name = ? AND record_key = ? RETURNING id",
        )
        .bind(unit)
        .bind(table)
        .bind(key)
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?;
        Some(id.ok_or_else(|| {
            PluginError::invalid_params("Cordis record to back up does not exist")
        })?)
    } else {
        None
    };
    sqlx::query("DELETE FROM cordis_records WHERE unit = ? AND table_name = ? AND record_key = ?")
        .bind(unit)
        .bind(table)
        .bind(key)
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(id)
}

async fn check_capacity(connection: &mut SqliteConnection) -> Result<()> {
    let (count, bytes) = sqlx::query_as::<_, (i64, i64)>(
        "SELECT count(*), coalesce(sum(length(CAST(value AS BLOB)) +
            length(CAST(record_key AS BLOB)) + length(table_name) + length(unit)), 0)
         FROM cordis_records",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    let units = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM cordis_units")
        .fetch_one(connection)
        .await
        .map_err(sql_error)?;
    if units > MAX_UNITS || count > MAX_ROWS || bytes > MAX_SNAPSHOT_BYTES {
        return Err(PluginError::invalid_params(
            "Cordis record store exceeds its snapshot capacity",
        ));
    }
    Ok(())
}

fn prepare_database_file(path: PathBuf) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("missing Cordis storage directory"))?;
    agena_storage_layout::secure_directory(parent)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut filename = path.as_os_str().to_owned();
        filename.push(suffix);
        let sidecar = PathBuf::from(filename);
        match std::fs::symlink_metadata(&sidecar) {
            Ok(_) => agena_storage_layout::secure_file(&sidecar)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    agena_storage_layout::secure_file(&path)
}

fn malformed() -> PluginError {
    PluginError::internal("invalid saved Cordis record store")
}

fn sql_error(error: sqlx::Error) -> PluginError {
    PluginError::internal_error(&error)
}
