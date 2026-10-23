//! Process tracing and database-connection settings shared by entry points.

use std::sync::Arc;

use agena_storage::{StorageConfig, StorageConfigError};
use sea_orm::DbErr;
use tracing_subscriber::EnvFilter;

use crate::DatabaseCompositionInputs;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_includes_database_and_adapter_directives() {
        let filter = runtime_env_filter(&RuntimeTracingConfiguration::default()).unwrap();
        let rendered = filter.to_string();
        assert!(rendered.contains("sqlx=error"));
        assert!(rendered.contains("sea_orm=error"));
    }

    #[tokio::test]
    async fn scheduler_shares_the_session_database() {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        let kernel = connect_runtime_database(DatabaseCompositionInputs {
            database_url: Some("sqlite::memory:".into()),
            database_path: None,
        })
        .await
        .unwrap()
        .unwrap();
        let row = kernel
            .database
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT name FROM sqlite_schema WHERE type='table' AND name='agena_scheduler_jobs'",
            ))
            .await
            .unwrap();
        assert!(row.is_some());
        kernel.ports.clone().shutdown().await.unwrap();
    }
}
