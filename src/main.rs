#[tokio::main]
async fn main() -> Result<(), agena::AppError> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    // Optional smoke check:
    // AGENA_PROVIDER=openai|anthropic
    // AGENA_MODEL=gpt-4.1-mini|claude-3-7-sonnet-latest
    // AGENA_PROMPT="Say hello"
    if let (Ok(provider_id), Ok(model), Ok(prompt)) = (
        std::env::var("AGENA_PROVIDER"),
        std::env::var("AGENA_MODEL"),
        std::env::var("AGENA_PROMPT"),
    ) {
        let registry = agena::provider::ProviderRegistry::with_defaults_from_env()?;
        let response = registry
            .complete(
                &provider_id,
                agena::provider::CompletionRequest {
                    model,
                    system: None,
                    messages: vec![agena::provider::ProviderMessage::new(
                        agena::role::Role::User,
                        prompt,
                    )],
                    temperature: None,
                    max_output_tokens: Some(256),
                },
            )
            .await?;

        tracing::info!(
            provider = %response.provider_id,
            model = %response.model,
            output = %response.text,
            "agena provider smoke request succeeded"
        );
    }

    Ok(())
}
