//! Process tracing and database-connection settings shared by entry points.

use std::{str::FromStr, sync::Arc};

use agena_storage::{StorageConfig, StorageConfigError};
use agena_storage_sqlite::initialize_schema;
use log::LevelFilter;
use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};
use tracing_subscriber::EnvFilter;

use crate::{DatabaseCompositionInputs, connect_or_initialize};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RuntimeTracingConfiguration {
    pub filter: String,
    pub database: String,
    pub adapter: String,
}

impl Default for RuntimeTracingConfiguration {
    fn default() -> Self {
        Self {
            filter: "info".to_owned(),
            database: "error".to_owned(),
            adapter: "off".to_owned(),
        }
    }
}

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

pub(crate) async fn connect_database(
    url: &str,
    config: &RuntimeTracingConfiguration,
) -> Result<DatabaseConnection, DbErr> {
    let mut options = ConnectOptions::new(url.to_owned());
    let level = database_log_level(config).unwrap_or(LevelFilter::Error);
    let enable_sqlx_statement_logging = matches!(
        level,
        LevelFilter::Warn | LevelFilter::Info | LevelFilter::Debug | LevelFilter::Trace
    );
    options.sqlx_logging(enable_sqlx_statement_logging);
    if enable_sqlx_statement_logging {
        options.sqlx_logging_level(level);
    }
    Database::connect(options).await
}

/// Errors produced while Runtime composes the process database connection.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RuntimeDatabaseCompositionError {
    #[error(transparent)]
    StorageConfig(#[from] StorageConfigError),
    #[error(transparent)]
    Database(#[from] DbErr),
}

/// Resolve, connect, and optionally initialize the concrete SQLite database.
///
/// Process entrypoints and Runtime composition supply only bootstrap inputs. The
/// Runtime owns URL resolution, connection reuse, parent-directory creation,
/// and schema initialization ordering.
pub(crate) async fn connect_runtime_database(
    inputs: DatabaseCompositionInputs<
        Option<Arc<DatabaseConnection>>,
        Option<String>,
        &RuntimeTracingConfiguration,
    >,
) -> Result<Option<Arc<DatabaseConnection>>, RuntimeDatabaseCompositionError> {
    let DatabaseCompositionInputs {
        database_connection,
        database_url,
        database_path,
        initialize_schema: should_initialize_schema,
        tracing,
    } = inputs;
    connect_or_initialize(
        database_connection,
        should_initialize_schema,
        || async move {
            let url = StorageConfig {
                database_url,
                database_path,
            }
            .resolve_url()?;
            StorageConfig::ensure_parent(url.as_str())?;
            connect_database(url.as_str(), tracing)
                .await
                .map_err(RuntimeDatabaseCompositionError::from)
        },
        |database| async move {
            initialize_schema(database.as_ref())
                .await
                .map_err(RuntimeDatabaseCompositionError::from)
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{RuntimeTracingConfiguration, connect_runtime_database, runtime_env_filter};
    use crate::DatabaseCompositionInputs;

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
            database_connection: None,
            database_url: Some("sqlite::memory:".to_owned()),
            database_path: None,
            initialize_schema: true,
            tracing: &RuntimeTracingConfiguration::default(),
        })
        .await
        .expect("compose in-memory runtime database");

        assert!(database.is_some());
    }
}
