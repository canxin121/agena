use std::str::FromStr;

use log::LevelFilter;
use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};
use tracing_subscriber::EnvFilter;

use crate::config::TracingConfig;

pub fn env_filter(
    config: &TracingConfig,
) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
    config.env_filter()
}

pub fn database_log_level(config: &TracingConfig) -> Result<LevelFilter, log::ParseLevelError> {
    LevelFilter::from_str(config.database_level.trim())
}

pub async fn connect_database(
    url: &str,
    config: &TracingConfig,
) -> Result<DatabaseConnection, DbErr> {
    let mut options = ConnectOptions::new(url.to_owned());
    let level = database_log_level(config).unwrap_or(LevelFilter::Error);
    // SQLx statement logging uses the configured level for every statement,
    // so "error" would otherwise print all SQL as ERROR. Treat error-or-less
    // as "disable statement logging" while still allowing library errors.
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
