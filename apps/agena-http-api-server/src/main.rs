use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use agena::{
    AppError,
    config::{ConfigLoader, ConfigModeName, ConfigOverride, LoadConfigRequest},
    runtime::AgenaRuntime,
    storage::StorageConfig,
};
use agena_http_api::{ApiState, router as v1_router};
use agena_api_server::{AppState as V2State, router as v2_router};
use axum::Router;
use clap::{Args, Parser, Subcommand};
use sea_orm::Database;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Clone, Parser)]
#[command(name = "agena-http-api", version, about = "Agena HTTP API server")]
struct AgenaHttpApiCli {
    #[arg(long, env = "AGENA_CONFIG", global = true)]
    config: Option<PathBuf>,
    #[arg(long, env = "AGENA_MODE", global = true)]
    mode: Option<ConfigModeName>,
    #[arg(short = 'c', long = "set", global = true)]
    overrides: Vec<ConfigOverride>,
    #[command(subcommand)]
    command: Option<HttpApiCommand>,
}

#[derive(Debug, Clone, Subcommand)]
enum HttpApiCommand {
    Serve(ServeCommand),
}

#[derive(Debug, Clone, Args)]
struct ServeCommand {
    #[arg(long, default_value = "127.0.0.1:8765")]
    listen: SocketAddr,
    #[arg(long, env = "AGENA_DATABASE_URL")]
    database_url: Option<String>,
    #[arg(long, env = "AGENA_DATABASE_PATH")]
    database_path: Option<PathBuf>,
    #[arg(long)]
    workspace_root: Option<PathBuf>,
}

impl AgenaHttpApiCli {
    fn load_request(&self) -> LoadConfigRequest {
        LoadConfigRequest {
            config_path: self.config.clone(),
            mode: self.mode.clone(),
            overrides: self.overrides.clone(),
        }
    }

    async fn run(self) -> Result<(), AppError> {
        let command = self
            .command
            .clone()
            .unwrap_or(HttpApiCommand::Serve(ServeCommand {
                listen: "127.0.0.1:8765"
                    .parse()
                    .expect("default listen address should parse"),
                database_url: None,
                database_path: None,
                workspace_root: None,
            }));
        match command {
            HttpApiCommand::Serve(command) => self.run_serve(command).await,
        }
    }

    async fn run_serve(self, command: ServeCommand) -> Result<(), AppError> {
        let storage = StorageConfig {
            database_url: command.database_url,
            database_path: command.database_path,
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;

        let db = Arc::new(Database::connect(database_url.as_str()).await?);
        let workspace_root = command.workspace_root.unwrap_or(env::current_dir()?);
        let runtime = AgenaRuntime::builder()
            .with_load_request(self.load_request())
            .with_workspace_root(workspace_root)
            .with_database_connection(db.as_ref().clone())
            .build()
            .await?;
        let listener = tokio::net::TcpListener::bind(command.listen).await?;

        tracing::info!(
            listen = %command.listen,
            database = %StorageConfig::display_location(database_url.as_str()),
            "Agena HTTP API server listening"
        );

        let v1 = v1_router(ApiState::new(runtime.clone(), db));
        let v2 = v2_router(V2State::new(runtime));
        let app = Router::new().merge(v1).merge(v2);

        axum::serve(listener, app)
            .await
            .map_err(|e| AppError::Internal(format!("axum::serve failed: {e}")))
    }
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let cli = AgenaHttpApiCli::parse();
    let filter = ConfigLoader::default()
        .load(&cli.load_request())
        .map(|resolution| resolution.config.tracing.filter)
        .unwrap_or_else(|_| "info".to_owned());

    let initial_filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact(),
        )
        .with(initial_filter)
        .init();

    cli.run().await
}
