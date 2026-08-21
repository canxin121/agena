//! Concrete AWS credential-chain adapter for Amazon Bedrock.
//!
//! Runtime-owned Bedrock request projection and signing use the public
//! credential value below, while this leaf is the only workspace package that
//! constructs an AWS SDK config/provider chain.

use aws_config::{BehaviorVersion, Region};
use aws_credential_types::provider::ProvideCredentials;
use aws_smithy_http_client::{
    Builder as AwsHttpClientBuilder,
    tls::{self, TlsContext, TrustStore, rustls_provider::CryptoMode},
};
use aws_smithy_runtime_api::client::http::SharedHttpClient;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

pub use aws_credential_types::Credentials as AwsCredentials;

#[derive(Debug, thiserror::Error)]
/// Error resolving Amazon Bedrock credentials.
pub enum BedrockCredentialError {
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

fn der_certificate_to_pem(der: &[u8]) -> Vec<u8> {
    let encoded = BASE64_STANDARD.encode(der);
    let mut pem = Vec::with_capacity(encoded.len() + 64);
    pem.extend_from_slice(b"-----BEGIN CERTIFICATE-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        pem.extend_from_slice(chunk);
        pem.push(b'\n');
    }
    pem.extend_from_slice(b"-----END CERTIFICATE-----\n");
    pem
}

fn aws_http_client() -> SharedHttpClient {
    let mut trust_store = TrustStore::empty();
    for cert in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
        trust_store.add_pem_certificate(der_certificate_to_pem(cert.as_ref()));
    }
    let tls_context = TlsContext::builder()
        .with_trust_store(trust_store)
        .build()
        .expect("static Mozilla root certificates form a valid AWS TLS context");

    AwsHttpClientBuilder::new()
        .tls_provider(tls::Provider::Rustls(CryptoMode::Ring))
        .tls_context(tls_context)
        .build_https()
}

async fn resolve_provider_chain(
    region: &str,
    profile: Option<&str>,
) -> Result<AwsCredentials, BedrockCredentialError> {
    let mut loader = aws_config::defaults(BehaviorVersion::latest())
        .http_client(aws_http_client())
        .region(Region::new(region.to_owned()));
    if let Some(profile) = profile.filter(|value| !value.trim().is_empty()) {
        loader = loader.profile_name(profile.to_owned());
    }
    let sdk_config = loader.load().await;
    let provider = sdk_config.credentials_provider().ok_or_else(|| {
        BedrockCredentialError::Resolve(
            "AWS credential provider chain returned no provider".to_owned(),
        )
    })?;
    provider
        .provide_credentials()
        .await
        .map_err(|error| BedrockCredentialError::Resolve(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_aws_https_client_constructs() {
        assert!(!webpki_root_certs::TLS_SERVER_ROOT_CERTS.is_empty());
        let _client = aws_http_client();
    }

    #[test]
    fn runtime_reqwest_https_client_constructs() {
        let _client = reqwest::Client::builder()
            .tls_backend_rustls()
            .build()
            .expect("reqwest Rustls/ring client with bundled WebPKI roots must construct");
    }
}
