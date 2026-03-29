use agena::{cli::AgenaCli, config::ConfigLoader};
use clap::Parser;

#[tokio::main]
async fn main() -> Result<(), agena::AppError> {
    let cli = AgenaCli::parse();
    let filter = ConfigLoader::default()
        .load(&cli.load_request())
        .map(|resolution| resolution.config.tracing.filter)
        .unwrap_or_else(|_| "info".to_owned());

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(filter)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    cli.run().await
}
