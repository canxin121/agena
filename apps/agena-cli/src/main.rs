use agena::{cli::AgenaCli, config::ConfigLoader};
use clap::Parser;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), agena::AppError> {
    let cli = AgenaCli::parse();
    let resolution = ConfigLoader::default().load(&cli.load_request()).ok();
    let filter = resolution
        .as_ref()
        .map(|resolution| resolution.config.tracing.filter.clone())
        .unwrap_or_else(|| "info".to_owned());
    let telemetry = resolution
        .as_ref()
        .map(|resolution| resolution.config.telemetry.clone())
        .unwrap_or_default();

    let initial_filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter_layer, filter_handle) = tracing_subscriber::reload::Layer::new(initial_filter);
    if let Some(telemetry) = agena_otel::build_layer(&telemetry)
        .map_err(|error| agena::AppError::Config(error.to_string()))?
    {
        let telemetry_layer = telemetry.layer();
        let _telemetry_guard = telemetry.guard;
        tracing_subscriber::registry()
            .with(filter_layer)
            .with(telemetry_layer)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .compact(),
            )
            .init();
        cli.run(Some(filter_handle)).await
    } else {
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
}
