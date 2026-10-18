//! Process tracing and database-connection settings shared by entry points.

use std::{str::FromStr, sync::Arc};

use agena_storage::{StorageConfig, StorageConfigError};
use log::LevelFilter;
use sea_orm::{DatabaseConnection, DbErr};
use tracing_subscriber::EnvFilter;

use crate::{DatabaseCompositionInputs, connect_or_initialize};
pub use agena_runtime_config::RuntimeTracingConfiguration;

pub fn runtime_env_filter(
    config: &RuntimeTracingConfiguration,
) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
    let mut filter = EnvFilter::try_new(config.filter.as_str())?;
    for target in ["sqlx", "sea_orm"] {
        filter = filter.add_directive(format!("{target}={}", config.database).parse()?);
    }
    filter = filter.add_directive(format!("{}::adapter={}", "agena", config.adapter).parse()?);
    Ok(filter)
}

/// Apply a resolved tracing policy through a Runtime control state's optional
/// reload handle. The caller retains only process-specific logging/reporting.
pub(crate) fn apply_runtime_tracing_filter<S, E>(
    control_state: &crate::RuntimeControlState<S, E>,
    config: &RuntimeTracingConfiguration,
) -> Result<bool, tracing_subscriber::filter::ParseError>
where
    E: Send + Sync + 'static,
{
    let filter = runtime_env_filter(config)?;
    Ok(control_state.reload_tracing_filter(filter))
}

pub(crate) fn database_log_level(
    config: &RuntimeTracingConfiguration,
) -> Result<LevelFilter, log::ParseLevelError> {
    LevelFilter::from_str(config.database.trim())
}

/// Whether a database URL refers to an ephemeral in-memory SQLite database.
///
/// The owned SQLite factory retains one memory connection for its lifetime.
pub(crate) fn is_in_memory_database(url: &str) -> bool {
    agena_storage_sqlite::is_memory_database(url)
}

pub(crate) async fn connect_database(
    url: &str,
    config: &RuntimeTracingConfiguration,
) -> Result<DatabaseConnection, DbErr> {
    let level = database_log_level(config).unwrap_or(LevelFilter::Error);
    let level = if level == LevelFilter::Error {
        LevelFilter::Off
    } else {
        level
    };
    agena_storage_sqlite::connect_owned_database(url, level).await
}

/// Errors produced while Runtime composes the process database connection.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RuntimeDatabaseCompositionError {
    #[error(transparent)]
    StorageConfig(#[from] StorageConfigError),
    #[error(transparent)]
    Database(#[from] DbErr),
}

/// Resolve, own, and validate the session kernel's exact SQLite schema.
///
/// Process entrypoints and Runtime composition supply only bootstrap inputs. The
/// Runtime owns URL resolution, lifetime ownership, parent-directory creation,
/// and schema initialization ordering.
pub(crate) async fn connect_runtime_database(
    inputs: DatabaseCompositionInputs,
) -> Result<Option<Arc<agena_storage_sqlite::SqliteKernel>>, RuntimeDatabaseCompositionError> {
    let DatabaseCompositionInputs {
        database_url,
        database_path,
    } = inputs;
    let url = StorageConfig {
        database_url,
        database_path,
    }
    .resolve_url()?;
    let kernel = agena_storage_sqlite::open_kernel_runtime(&url)
        .await
        .map_err(|error| {
            RuntimeDatabaseCompositionError::Database(DbErr::Custom(error.to_string()))
        })?;
    Ok(Some(Arc::new(kernel)))
}

/// Resolve the chat database URL exactly as `connect_runtime_database` does,
/// so callers can decide whether the scheduler database should be file-backed.
pub(crate) fn resolve_runtime_database_url(
    database_url: Option<String>,
    database_path: Option<std::path::PathBuf>,
) -> Result<String, StorageConfigError> {
    StorageConfig {
        database_url,
        database_path,
    }
    .resolve_url()
}

/// Conventional scheduler database location (`~/.agena/scheduler.db`), a
/// sibling of the chat database's conventional path.
fn scheduler_default_path() -> std::path::PathBuf {
    let mut base = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    base.push("agena");
    base.push("scheduler.db");
    base
}

/// Resolve the scheduler database URL from (in priority order): an explicit
/// URL or path, `AGENA_SCHEDULER_DATABASE_URL`/`AGENA_SCHEDULER_DATABASE_PATH`,
/// or the conventional scheduler default path.
pub(crate) fn resolve_scheduler_database_url(
    database_url: Option<String>,
    database_path: Option<std::path::PathBuf>,
) -> Result<String, StorageConfigError> {
    if let Some(url) = database_url.or_else(|| std::env::var("AGENA_SCHEDULER_DATABASE_URL").ok()) {
        return Ok(url);
    }
    let path = database_path
        .or_else(|| {
            std::env::var("AGENA_SCHEDULER_DATABASE_PATH")
                .ok()
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(scheduler_default_path);
    Ok(format!("sqlite://{}?mode=rwc", path.display()))
}

/// Connect and initialize the dedicated scheduler SQLite database.
///
/// Mirrors `connect_runtime_database` but targets the scheduler schema and the
/// scheduler database location. Each process acquires the same owned factory;
/// no raw injected pool can bypass lifecycle ownership.
pub(crate) async fn connect_scheduler_database(
    database_url: Option<String>,
    database_path: Option<std::path::PathBuf>,
    tracing: &RuntimeTracingConfiguration,
) -> Result<Option<Arc<DatabaseConnection>>, RuntimeDatabaseCompositionError> {
    connect_or_initialize(
        None,
        true,
        || async move {
            let url = resolve_scheduler_database_url(database_url, database_path)?;
            connect_database(url.as_str(), tracing)
                .await
                .map_err(RuntimeDatabaseCompositionError::from)
        },
        |database| async move {
            agena_scheduler::schema::initialize_schema(database.as_ref())
                .await
                .map_err(RuntimeDatabaseCompositionError::from)
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{RuntimeTracingConfiguration, connect_runtime_database, runtime_env_filter};
    use crate::{DatabaseCompositionInputs, connect_scheduler_database};

    #[test]
    fn default_filter_includes_database_and_adapter_directives() {
        let filter = runtime_env_filter(&RuntimeTracingConfiguration::default())
            .expect("default runtime tracing filter");
        let rendered = filter.to_string();
        assert!(rendered.contains("sqlx=error"));
        assert!(rendered.contains("sea_orm=error"));
        assert!(rendered.contains(&format!("{}::adapter=off", "agena")));
    }

    #[tokio::test]
    async fn runtime_database_composition_initializes_an_in_memory_database() {
        let database = connect_runtime_database(DatabaseCompositionInputs {
            database_url: Some("sqlite::memory:".to_owned()),
            database_path: None,
        })
        .await
        .expect("compose in-memory runtime database");

        assert!(database.is_some());
    }

    #[tokio::test]
    async fn scheduler_database_composition_initializes_its_own_schema() {
        use sea_orm::{ConnectionTrait, Statement};
        use std::path::PathBuf;

        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("scheduler.db");

        let database = connect_scheduler_database(
            None,
            Some(PathBuf::from(&db_path)),
            &RuntimeTracingConfiguration::default(),
        )
        .await
        .expect("compose scheduler database")
        .expect("scheduler database present");

        for table in ["agena_scheduler_jobs", "agena_scheduler_history"] {
            let count: i64 = database
                .query_one(Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    format!(
                        "SELECT COUNT(*) AS count FROM sqlite_master \
                         WHERE type = 'table' AND name = '{table}'"
                    ),
                ))
                .await
                .expect("query table")
                .expect("table row")
                .try_get("", "count")
                .expect("count value");
            assert_eq!(count, 1, "scheduler table {table} must exist");
        }
        // Reopening the scheduler database validates the exact current DDL;
        // there is deliberately no schema-version/migration contract.
        agena_scheduler::schema::initialize_schema(database.as_ref())
            .await
            .expect("reopen exact scheduler schema");
    }
}
