//! # agena-provider-bedrock-signing
//!
//! Concrete AWS SigV4 signing adapter for Amazon Bedrock requests.
//!
//! Computes the SigV4 signed headers ([`signed_headers`]) for Bedrock API
//! calls, with typed errors ([`BedrockSigningError`]).

use agena_provider_bedrock_auth::AwsCredentials;
use aws_sigv4::{
    http_request::{SignableBody, SignableRequest, SigningSettings, sign},
    sign::v4,
};

#[derive(Debug, thiserror::Error)]
pub enum BedrockSigningError {
    #[error("signing parameters: {0}")]
    SigningParameters(String),
    #[error("signable request: {0}")]
    SignableRequest(String),
    #[error("request signing: {0}")]
    Signing(String),
    #[error("signing request construction: {0}")]
    RequestConstruction(String),
    #[error("invalid header name `{name}`: {error}")]
    HeaderName { name: String, error: String },
    #[error("invalid header value for `{name}`: {error}")]
    HeaderValue { name: String, error: String },
}

/// Apply Bedrock's AWS SigV4 signature to a request and return the complete
/// header map. The caller owns request dispatch and provider-specific logging.
pub fn signed_headers(
    method: &str,
    url: &str,
    body: &[u8],
    headers: &[(String, String)],
    credentials: &AwsCredentials,
    region: &str,
) -> Result<http::HeaderMap, BedrockSigningError> {
    let identity = credentials.clone().into();
    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name("bedrock")
        .time(std::time::SystemTime::now())
        .settings(SigningSettings::default())
        .build()
        .map_err(|error| BedrockSigningError::SigningParameters(error.to_string()))?;
    let signable_request = SignableRequest::new(
        method,
        url,
        headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        SignableBody::Bytes(body),
    )
    .map_err(|error| BedrockSigningError::SignableRequest(error.to_string()))?;
    let (instructions, _) = sign(signable_request, &signing_params.into())
        .map_err(|error| BedrockSigningError::Signing(error.to_string()))?
        .into_parts();
    let mut request = http::Request::builder()
        .method(method)
        .uri(url)
        .body(())
        .map_err(|error| BedrockSigningError::RequestConstruction(error.to_string()))?;
    for (name, value) in headers {
        request.headers_mut().insert(
            http::header::HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                BedrockSigningError::HeaderName {
                    name: name.clone(),
                    error: error.to_string(),
                }
            })?,
            http::header::HeaderValue::from_str(value).map_err(|error| {
                BedrockSigningError::HeaderValue {
                    name: name.clone(),
                    error: error.to_string(),
                }
            })?,
        );
    }
    instructions.apply_to_request_http1x(&mut request);
    Ok(request.headers().clone())
}
