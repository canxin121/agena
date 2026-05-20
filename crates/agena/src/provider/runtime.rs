use std::time::Duration;

use crate::error::AppError;

const DEFAULT_PROVIDER_HTTP_TIMEOUT_SECS: u64 = 120;
const DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_PROVIDER_USER_AGENT: &str = concat!("agena/", env!("CARGO_PKG_VERSION"));
const DEFAULT_PROVIDER_REQUEST_MAX_RETRIES: u32 = 5;
const DEFAULT_PROVIDER_RETRY_BASE_DELAY_MS: u64 = 250;
const DEFAULT_PROVIDER_RETRY_MAX_DELAY_MS: u64 = 2_000;
const DEFAULT_PROVIDER_STREAM_REPLAY_MAX_RETRIES: u32 = 5;
const DEFAULT_PROVIDER_STREAM_REPLAY_MAX_EVENTS: usize = 2048;

#[derive(Debug, Clone, Copy)]
pub struct ProviderHttpClientConfig {
    pub timeout: Duration,
    pub connect_timeout: Duration,
}

impl Default for ProviderHttpClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_PROVIDER_HTTP_TIMEOUT_SECS),
            connect_timeout: Duration::from_secs(DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS),
        }
    }
}

impl ProviderHttpClientConfig {
    pub fn build_client(self) -> Result<reqwest::Client, AppError> {
        if self.timeout.is_zero() {
            return Err(AppError::Config(
                "provider http timeout must be greater than 0".to_owned(),
            ));
        }
        if self.connect_timeout.is_zero() {
            return Err(AppError::Config(
                "provider connect timeout must be greater than 0".to_owned(),
            ));
        }

        reqwest::Client::builder()
            .timeout(self.timeout)
            .connect_timeout(self.connect_timeout)
            .user_agent(DEFAULT_PROVIDER_USER_AGENT)
            .build()
            .map_err(AppError::from)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderRequestRetryConfig {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for ProviderRequestRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_PROVIDER_REQUEST_MAX_RETRIES,
            base_delay: Duration::from_millis(DEFAULT_PROVIDER_RETRY_BASE_DELAY_MS),
            max_delay: Duration::from_millis(DEFAULT_PROVIDER_RETRY_MAX_DELAY_MS),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderStreamReplayConfig {
    pub max_retries_after_output: u32,
    pub max_tracked_events: usize,
}

impl Default for ProviderStreamReplayConfig {
    fn default() -> Self {
        Self {
            max_retries_after_output: DEFAULT_PROVIDER_STREAM_REPLAY_MAX_RETRIES,
            max_tracked_events: DEFAULT_PROVIDER_STREAM_REPLAY_MAX_EVENTS,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderRuntimeConfig {
    pub request_retry: ProviderRequestRetryConfig,
    pub stream_replay: ProviderStreamReplayConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn provider_http_client_sets_agena_user_agent() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/ua")
            .match_header("user-agent", DEFAULT_PROVIDER_USER_AGENT)
            .with_status(200)
            .create_async()
            .await;

        let client = ProviderHttpClientConfig::default()
            .build_client()
            .expect("client should build");
        let response = client
            .get(format!("{}/ua", server.url()))
            .send()
            .await
            .expect("request should succeed");

        assert!(response.status().is_success());
    }
}
