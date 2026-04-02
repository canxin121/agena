use agena::{cli::AgenaCli, config::ConfigLoader};
use clap::Parser;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), agena::AppError> {
    let cli = AgenaCli::parse();
    let filter = ConfigLoader::default()
        .load(&cli.load_request())
        .map(|resolution| resolution.config.tracing.filter)
        .unwrap_or_else(|_| "info".to_owned());

    let initial_filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter_layer, filter_handle) = tracing_subscriber::reload::Layer::new(initial_filter);
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact(),
        )
        .init();

    cli.run(Some(filter_handle)).await
}
