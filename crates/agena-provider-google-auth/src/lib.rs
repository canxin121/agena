//! Concrete Google Application Default Credentials adapter.
//!
//! This leaf owns the `gcp_auth` SDK interaction. Higher layers retain their
//! credential cache, provider-specific provenance, and error presentation but
//! do not compile or name Google SDK types directly.

const GOOGLE_CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Debug, thiserror::Error)]
pub enum GoogleAdcError {
    #[error("initializing Google ADC provider: {0}")]
    Provider(String),
    #[error("obtaining Google ADC access token: {0}")]
    Token(String),
}

/// Resolve a Google ADC access token with the scope required by Google model
/// provider adapters.
pub async fn access_token() -> Result<String, GoogleAdcError> {
    let provider = gcp_auth::provider()
        .await
        .map_err(|error| GoogleAdcError::Provider(error.to_string()))?;
    let token = provider
        .token(&[GOOGLE_CLOUD_PLATFORM_SCOPE])
        .await
        .map_err(|error| GoogleAdcError::Token(error.to_string()))?;
    Ok(token.as_str().to_owned())
}
