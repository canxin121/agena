#[tokio::main]
async fn main() -> Result<(), agena::AppError> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    tracing::info!(
        "Agena started. Provider registration is explicit-only; build reqwest client and register providers in code."
    );

    Ok(())
}
