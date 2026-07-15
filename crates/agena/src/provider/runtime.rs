use std::{sync::LazyLock, time::Duration};

use crate::error::AppError;
use parking_lot::RwLock;
use serde::Deserialize;

const DEFAULT_PROVIDER_HTTP_TIMEOUT_SECS: u64 = 120;
const DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS: u64 = 15;

pub const CODEX_ORIGINATOR: &str = "codex_cli_rs";
pub const CODEX_MCP_CLIENT_NAME: &str = "codex-mcp-client";
pub const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.144.4";
pub const DEFAULT_CLAUDE_CLIENT_VERSION: &str = "2.1.209";
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
pub async fn fetch_latest_provider_client_versions() -> Result<ProviderClientVersions, AppError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CLIENT_VERSION_FETCH_TIMEOUT_SECS))
        .timeout(Duration::from_secs(CLIENT_VERSION_FETCH_TIMEOUT_SECS))
        .build()?;
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
    let os_info = os_info::get();
    let terminal = codex_terminal_user_agent();
    sanitize_http_user_agent(format!(
        "{CODEX_ORIGINATOR}/{version} ({} {}; {}) {terminal}",
        os_info.os_type(),
        os_info.version(),
        os_info.architecture().unwrap_or("unknown"),
    ))
}

fn codex_terminal_user_agent() -> String {
    let term_program = non_empty_env("TERM_PROGRAM");
    let term_program_version = non_empty_env("TERM_PROGRAM_VERSION");
    if let Some(program) = term_program {
        return term_program_version
            .map(|version| format!("{program}/{version}"))
            .unwrap_or(program);
    }
    non_empty_env("TERM").unwrap_or_else(|| "unknown".to_owned())
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn sanitize_http_user_agent(value: String) -> String {
    value
        .chars()
        .map(|character| {
            if matches!(character, ' '..='~') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

pub fn claude_code_api_user_agent() -> String {
    format!(
        "claude-cli/{} (external, cli)",
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
        "GeminiCLI/{version}/{model} ({}; {}; terminal)",
        gemini_node_platform(),
        gemini_node_architecture()
    )
}

fn gemini_node_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        platform => platform,
    }
}

fn gemini_node_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        "powerpc64" => "ppc64",
        architecture => architecture,
    }
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

#[cfg(test)]
mod tests {
    use super::{
        CODEX_ORIGINATOR, claude_code_api_user_agent, codex_user_agent, gemini_cli_user_agent,
        gemini_node_architecture, gemini_node_platform, latest_version_result,
        valid_client_version,
    };

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

    #[test]
    fn codex_user_agent_uses_the_official_originator_and_shape() {
        let user_agent = codex_user_agent();
        assert!(user_agent.starts_with(&format!("{CODEX_ORIGINATOR}/")));
        assert!(user_agent.contains(" ("));
        assert!(user_agent.contains("; "));
        assert!(user_agent.is_ascii());
        assert!(!user_agent.to_ascii_lowercase().contains("agena"));
    }

    #[test]
    fn claude_user_agent_uses_the_official_external_cli_identity() {
        let user_agent = claude_code_api_user_agent();
        assert!(user_agent.starts_with("claude-cli/"));
        assert!(user_agent.ends_with(" (external, cli)"));
        assert!(!user_agent.contains("agena"));
    }

    #[test]
    fn gemini_user_agent_uses_the_official_node_platform_shape() {
        let user_agent = gemini_cli_user_agent("gemini-3.1-pro-preview");
        assert!(user_agent.starts_with("GeminiCLI/"));
        assert!(user_agent.contains("/gemini-3.1-pro-preview ("));
        assert!(user_agent.contains(&format!(
            "({}; {}; terminal)",
            gemini_node_platform(),
            gemini_node_architecture()
        )));
        assert!(!user_agent.contains("agena"));
    }
}
