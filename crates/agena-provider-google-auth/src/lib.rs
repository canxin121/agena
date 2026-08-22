//! Concrete Google Application Default Credentials adapter.
//!
//! This leaf owns the `gcp_auth` SDK interaction. Higher layers retain their
//! credential cache, provider-specific provenance, and error presentation but
//! do not compile or name Google SDK types directly.

const GOOGLE_CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Debug, thiserror::Error)]
/// Error resolving Google Application Default Credentials.
pub enum GoogleAdcError {
    #[error("initializing Google ADC provider: {0}")]
    Provider(String),
    #[error("obtaining Google ADC access token: {0}")]
    Token(String),
}

/// Resolve a Google ADC access token with the scope required by Google model
/// provider adapters.
#[cfg(not(all(target_arch = "aarch64", target_endian = "big")))]
pub async fn access_token() -> Result<String, GoogleAdcError> {
    let provider = gcp_auth::provider().await.map_err(|error| {
        GoogleAdcError::Provider(agena_failure::diagnostic::format_error_chain_with_context(
            "failed to initialize the Google ADC provider",
            &error,
        ))
    })?;
    let token = provider
        .token(&[GOOGLE_CLOUD_PLATFORM_SCOPE])
        .await
        .map_err(|error| {
            GoogleAdcError::Token(agena_failure::diagnostic::format_error_chain_with_context(
                "failed to obtain a Google ADC access token",
                &error,
            ))
        })?;
    Ok(token.as_str().to_owned())
}

#[cfg(all(target_arch = "aarch64", target_endian = "big"))]
pub async fn access_token() -> Result<String, GoogleAdcError> {
    Err(GoogleAdcError::Provider(
        "Google ADC is unavailable on big-endian AArch64 builds".to_owned(),
    ))
}
