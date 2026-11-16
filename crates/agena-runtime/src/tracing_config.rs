//! Process tracing and database-connection settings shared by entry points.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

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
    config_path: PathBuf,
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
    let resolver: Arc<agena_storage_sqlite::NetworkMigrationResolver> =
        Arc::new(move |workspace| {
            let workspace_path = workspace
                .map(|path| agena_runtime_config::project_config_path(Path::new(path)))
                .unwrap_or_else(|| config_path.clone());
            agena_runtime_config::network_config_migration::migrate_network_config_files(
                &config_path,
                &workspace_path,
            )
            .map(|migration| migration.effective)
            .map_err(|error| error.to_string())
        });
    let kernel = agena_storage_sqlite::open_kernel_runtime_with_network_migration(&url, resolver)
        .await
        .map_err(|error| {
            RuntimeDatabaseCompositionError::Database(DbErr::Custom(error.to_string()))
        })?;
    Ok(Some(Arc::new(kernel)))
}
