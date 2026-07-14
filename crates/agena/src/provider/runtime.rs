use std::{sync::LazyLock, time::Duration};

use parking_lot::RwLock;
use serde::Deserialize;

use crate::error::AppError;

const DEFAULT_PROVIDER_HTTP_TIMEOUT_SECS: u64 = 120;
const DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_PROVIDER_REQUEST_MAX_RETRIES: u32 = 5;
const DEFAULT_PROVIDER_RETRY_BASE_DELAY_MS: u64 = 250;
const DEFAULT_PROVIDER_RETRY_MAX_DELAY_MS: u64 = 2_000;
const DEFAULT_PROVIDER_STREAM_REPLAY_MAX_RETRIES: u32 = 5;
const DEFAULT_PROVIDER_STREAM_REPLAY_MAX_EVENTS: usize = 2048;

pub const CODEX_ORIGINATOR: &str = "codex_cli_rs";
pub const CODEX_MCP_CLIENT_NAME: &str = "codex-mcp-client";
pub const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.144.3";
pub const DEFAULT_CLAUDE_CLIENT_VERSION: &str = "2.1.208";
pub const DEFAULT_GEMINI_CLIENT_VERSION: &str = "0.50.0";
const CLIENT_VERSION_FETCH_TIMEOUT_SECS: u64 = 5;
const CODEX_VERSION_URL: &str = "https://registry.npmjs.org/@openai%2Fcodex/latest";
const CLAUDE_VERSION_URL: &str = "https://registry.npmjs.org/@anthropic-ai%2Fclaude-code/latest";
const GEMINI_VERSION_URL: &str = "https://registry.npmjs.org/@google%2Fgemini-cli/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderClientVersions {
    pub codex: String,
    pub claude: String,
    pub gemini: String,
}

impl Default for ProviderClientVersions {
    fn default() -> Self {
        Self {
            codex: DEFAULT_CODEX_CLIENT_VERSION.to_owned(),
            claude: DEFAULT_CLAUDE_CLIENT_VERSION.to_owned(),
            gemini: DEFAULT_GEMINI_CLIENT_VERSION.to_owned(),
        }
    }
}

static ACTIVE_CLIENT_VERSIONS: LazyLock<RwLock<ProviderClientVersions>> =
    LazyLock::new(|| RwLock::new(ProviderClientVersions::default()));

pub fn provider_client_versions() -> ProviderClientVersions {
    ACTIVE_CLIENT_VERSIONS.read().clone()
}

pub fn apply_provider_client_version_settings(
    settings: &crate::config::ProviderClientVersionSettings,
) {
    *ACTIVE_CLIENT_VERSIONS.write() = ProviderClientVersions {
        codex: settings.codex.clone(),
        claude: settings.claude.clone(),
        gemini: settings.gemini.clone(),
    };
}

/// Fetch the latest compatible CLI versions from npm on explicit user request.
///
/// Runtime construction and reload deliberately do not call this function. The
/// caller is responsible for persisting the returned exact versions before
/// applying them.
pub async fn fetch_latest_provider_client_versions() -> Result<ProviderClientVersions, AppError> {
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CLIENT_VERSION_FETCH_TIMEOUT_SECS))
        .timeout(Duration::from_secs(CLIENT_VERSION_FETCH_TIMEOUT_SECS))
        .user_agent(format!("agena/{}", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(error) => return Err(AppError::Http(error)),
    };

    let (codex, claude, gemini) = tokio::join!(
        fetch_npm_package_version(&client, CODEX_VERSION_URL),
        fetch_npm_package_version(&client, CLAUDE_VERSION_URL),
        fetch_npm_package_version(&client, GEMINI_VERSION_URL),
    );
    Ok(ProviderClientVersions {
        codex: latest_version_result("@openai/codex", codex)?,
        claude: latest_version_result("@anthropic-ai/claude-code", claude)?,
        gemini: latest_version_result("@google/gemini-cli", gemini)?,
    })
}

fn latest_version_result(
    package: &str,
    result: Result<String, String>,
) -> Result<String, AppError> {
    result.map_err(|error| {
        AppError::Provider(format!(
            "failed to fetch latest {package} client version: {error}"
        ))
    })
}

#[derive(Debug, Deserialize)]
struct NpmPackageVersion {
    version: String,
}

async fn fetch_npm_package_version(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let payload = response
        .json::<NpmPackageVersion>()
        .await
        .map_err(|error| error.to_string())?;
    let version = payload.version.trim();
    if !valid_client_version(version) {
        return Err("registry returned an invalid version".to_owned());
    }
    Ok(version.to_owned())
}

fn valid_client_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 128
        && version
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+_".contains(character))
}

pub fn codex_package_version() -> String {
    provider_client_versions().codex
}

pub fn codex_user_agent() -> String {
    let version = codex_package_version();
    format!(
        "codex_cli_rs/{version} ({}; {}) rust",
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
    format!(
        "claude-cli/{} ({user_type}, {entrypoint})",
        provider_client_versions().claude
    )
}

pub fn claude_code_user_agent() -> String {
    format!("claude-code/{}", provider_client_versions().claude)
}

pub fn claude_user_web_fetch_user_agent() -> String {
    format!(
        "Claude-User ({}; +https://support.anthropic.com/)",
        claude_code_user_agent()
    )
}

pub fn gemini_cli_user_agent(model: &str) -> String {
    let version = provider_client_versions().gemini;
    format!(
        "GeminiCLI/{version}/{model} ({}; {}; cli)",
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
    use super::{latest_version_result, valid_client_version};

    #[test]
    fn registry_versions_accept_prereleases_but_reject_header_injection() {
        assert!(valid_client_version("0.51.0-nightly.20260625.g3fbf93e26"));
        assert!(valid_client_version("2.1.207"));
        assert!(!valid_client_version("2.1.207\r\nX-Test: injected"));
        assert!(!valid_client_version(""));
    }

    #[test]
    fn manual_registry_errors_identify_the_failed_package() {
        let error = latest_version_result("@openai/codex", Err("offline".to_owned()))
            .expect_err("registry failure should be returned");
        assert!(error.to_string().contains("@openai/codex"));
        assert!(error.to_string().contains("offline"));
    }
}
