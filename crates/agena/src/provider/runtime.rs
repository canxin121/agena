use std::time::Duration;

use crate::error::AppError;

const DEFAULT_PROVIDER_HTTP_TIMEOUT_SECS: u64 = 120;
const DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_PROVIDER_REQUEST_MAX_RETRIES: u32 = 5;
const DEFAULT_PROVIDER_RETRY_BASE_DELAY_MS: u64 = 250;
const DEFAULT_PROVIDER_RETRY_MAX_DELAY_MS: u64 = 2_000;
const DEFAULT_PROVIDER_STREAM_REPLAY_MAX_RETRIES: u32 = 5;
const DEFAULT_PROVIDER_STREAM_REPLAY_MAX_EVENTS: usize = 2048;

pub const CLAUDE_CODE_VERSION: &str = "2.1.88";
pub const CLAUDE_CODE_USER_AGENT: &str = "claude-code/2.1.88";
pub const CLAUDE_CODE_API_USER_AGENT: &str = "claude-cli/2.1.88 (external, cli)";
pub const CODEX_ORIGINATOR: &str = "codex_cli_rs";
pub const CODEX_PACKAGE_VERSION: &str = "0.142.4";
pub const CODEX_MCP_CLIENT_NAME: &str = "codex-mcp-client";
pub const CODEX_USER_AGENT: &str = "codex_cli_rs/0.142.4";
pub const GEMINI_CLI_VERSION: &str = "0.51.0-nightly.20260625.g3fbf93e26";
pub const GEMINI_CLI_USER_AGENT_PREFIX: &str = "GeminiCLI/0.51.0-nightly.20260625.g3fbf93e26";
pub const CLAUDE_USER_WEB_FETCH_USER_AGENT: &str =
    "Claude-User (claude-code/2.1.88; +https://support.anthropic.com/)";

pub fn codex_user_agent() -> String {
    format!(
        "{CODEX_USER_AGENT} ({}; {}) rust",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

pub fn claude_code_api_user_agent() -> String {
    let user_type = std::env::var("USER_TYPE")
        .ok()
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .unwrap_or_else(|| "external".to_owned());
    let entrypoint = std::env::var("CLAUDE_CODE_ENTRYPOINT")
        .ok()
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .unwrap_or_else(|| "cli".to_owned());
    format!("claude-cli/{CLAUDE_CODE_VERSION} ({user_type}, {entrypoint})")
}

pub fn claude_code_user_agent() -> String {
    CLAUDE_CODE_USER_AGENT.to_owned()
}

pub fn claude_user_web_fetch_user_agent() -> String {
    format!(
        "Claude-User ({}; +https://support.anthropic.com/)",
        claude_code_user_agent()
    )
}

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
