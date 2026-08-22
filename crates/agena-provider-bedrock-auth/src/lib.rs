//! Concrete AWS credential-chain adapter for Amazon Bedrock.
//!
//! Runtime-owned Bedrock request projection and signing use the public
//! credential value below, while this leaf is the only workspace package that
//! constructs an AWS SDK config/provider chain.

#[cfg(not(any(
    all(target_arch = "aarch64", target_endian = "big"),
    all(target_arch = "x86", target_os = "windows", target_env = "gnu")
)))]
use aws_config::{BehaviorVersion, Region};
#[cfg(not(any(
    all(target_arch = "aarch64", target_endian = "big"),
    all(target_arch = "x86", target_os = "windows", target_env = "gnu")
)))]
use aws_credential_types::provider::ProvideCredentials;

pub use aws_credential_types::Credentials as AwsCredentials;

#[derive(Debug, thiserror::Error)]
/// Error resolving Amazon Bedrock credentials.
pub enum BedrockCredentialError {
    #[error("AWS credential provider chain is unavailable")]
    ProviderUnavailable,
    #[error("resolving AWS credentials from the provider chain: {0}")]
    Resolve(String),
}

/// Resolve static credentials when configured, otherwise use the standard AWS
/// provider chain with the configured region and optional named profile.
pub async fn resolve_credentials(
    region: &str,
    profile: Option<&str>,
    static_credentials: Option<&AwsCredentials>,
) -> Result<AwsCredentials, BedrockCredentialError> {
    if let Some(credentials) = static_credentials {
        return Ok(credentials.clone());
    }

    resolve_provider_chain(region, profile).await
}

/// Construct static credentials parsed from Agena configuration values.
pub fn static_credentials(
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
) -> AwsCredentials {
    AwsCredentials::new(
        access_key_id,
        secret_access_key,
        session_token,
        None,
        "agena-config",
    )
}

#[cfg(not(any(
    all(target_arch = "aarch64", target_endian = "big"),
    all(target_arch = "x86", target_os = "windows", target_env = "gnu")
)))]
async fn resolve_provider_chain(
    region: &str,
    profile: Option<&str>,
) -> Result<AwsCredentials, BedrockCredentialError> {
    let mut loader =
        aws_config::defaults(BehaviorVersion::latest()).region(Region::new(region.to_owned()));
    if let Some(profile) = profile.filter(|value| !value.trim().is_empty()) {
        loader = loader.profile_name(profile.to_owned());
    }
    let sdk_config = loader.load().await;
    let provider = sdk_config
        .credentials_provider()
        .ok_or(BedrockCredentialError::ProviderUnavailable)?;
    provider.provide_credentials().await.map_err(|error| {
        BedrockCredentialError::Resolve(agena_failure::diagnostic::format_error_chain_with_context(
            "failed to resolve AWS credentials for Amazon Bedrock",
            &error,
        ))
    })
}

#[cfg(any(
    all(target_arch = "aarch64", target_endian = "big"),
    all(target_arch = "x86", target_os = "windows", target_env = "gnu")
))]
async fn resolve_provider_chain(
    _region: &str,
    _profile: Option<&str>,
) -> Result<AwsCredentials, BedrockCredentialError> {
    Err(BedrockCredentialError::ProviderUnavailable)
}
