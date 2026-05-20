use std::time::Duration;

use crate::error::AppError;

const DEFAULT_PROVIDER_HTTP_TIMEOUT_SECS: u64 = 120;
const DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_PROVIDER_REQUEST_MAX_RETRIES: u32 = 5;
const DEFAULT_PROVIDER_RETRY_BASE_DELAY_MS: u64 = 250;
const DEFAULT_PROVIDER_RETRY_MAX_DELAY_MS: u64 = 2_000;
const DEFAULT_PROVIDER_STREAM_REPLAY_MAX_RETRIES: u32 = 5;
const DEFAULT_PROVIDER_STREAM_REPLAY_MAX_EVENTS: usize = 2048;

pub const ATOMCODE_USER_AGENT: &str = "atomcode/4.18.1";
pub const CLAUDE_CODE_USER_AGENT: &str = "claude-code/2.1.145";
pub const CLAUDE_CODE_API_USER_AGENT: &str = "claude-cli/2.1.145 (external, cli)";
pub const CODEX_ORIGINATOR: &str = "codex_cli_rs";
pub const CODEX_PACKAGE_VERSION: &str = "0.132.0";
pub const CODEX_MCP_CLIENT_NAME: &str = "codex-mcp-client";
pub const CODEX_USER_AGENT: &str = "codex_cli_rs/0.132.0";
pub const GEMINI_CLI_USER_AGENT_PREFIX: &str = "GeminiCLI/0.42.0";
pub const CLAUDE_USER_WEB_FETCH_USER_AGENT: &str =
    "Claude-User (claude-code/2.1.145; +https://support.anthropic.com/)";

pub fn gemini_cli_user_agent(model: &str) -> String {
    format!(
        "{}/{model} ({}; {}; cli)",
        GEMINI_CLI_USER_AGENT_PREFIX,
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

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

    #[test]
    fn official_identity_headers_do_not_include_agena_name_or_version() {
        let values = [
            (ATOMCODE_USER_AGENT, "atomcode/4.18.1"),
            (CLAUDE_CODE_USER_AGENT, "claude-code/2.1.145"),
            (
                CLAUDE_CODE_API_USER_AGENT,
                "claude-cli/2.1.145 (external, cli)",
            ),
            (CODEX_ORIGINATOR, "codex_cli_rs"),
            (CODEX_PACKAGE_VERSION, "0.132.0"),
            (CODEX_MCP_CLIENT_NAME, "codex-mcp-client"),
            (CODEX_USER_AGENT, "codex_cli_rs/0.132.0"),
            (GEMINI_CLI_USER_AGENT_PREFIX, "GeminiCLI/0.42.0"),
            (
                CLAUDE_USER_WEB_FETCH_USER_AGENT,
                "Claude-User (claude-code/2.1.145; +https://support.anthropic.com/)",
            ),
        ];

        for (value, expected) in values {
            assert_eq!(value, expected);
            assert!(
                !value.to_ascii_lowercase().contains("agena"),
                "identity value leaked agena: {value}"
            );
        }

        let gemini = gemini_cli_user_agent("gemini-3-pro-preview");
        assert!(!gemini.to_ascii_lowercase().contains("agena"));
        assert!(gemini.starts_with("GeminiCLI/0.42.0/gemini-3-pro-preview "));
    }

    #[tokio::test]
    async fn provider_http_client_does_not_set_agena_user_agent() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/ua")
            .match_header("user-agent", mockito::Matcher::Missing)
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
