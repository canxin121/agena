//! Explicit Runtime-owned provider-client version refresh.

use std::sync::LazyLock;
use std::time::Duration;

use agena_provider::ProviderClientVersions;
use parking_lot::RwLock;
use serde::Deserialize;

const CLIENT_VERSION_FETCH_TIMEOUT_SECS: u64 = 5;
const CODEX_VERSION_URL: &str = "https://registry.npmjs.org/@openai%2Fcodex/latest";
const CLAUDE_VERSION_URL: &str = "https://registry.npmjs.org/@anthropic-ai%2Fclaude-code/latest";
const GEMINI_VERSION_URL: &str = "https://registry.npmjs.org/@google%2Fgemini-cli/latest";

/// Stable client identity used by Runtime-owned MCP and provider transports.
pub const RUNTIME_CODEX_MCP_CLIENT_NAME: &str = "codex-mcp-client";

static ACTIVE_CLIENT_VERSIONS: LazyLock<RwLock<ProviderClientVersions>> =
    LazyLock::new(|| RwLock::new(ProviderClientVersions::default()));

pub fn provider_client_versions() -> ProviderClientVersions {
    ACTIVE_CLIENT_VERSIONS.read().clone()
}

/// Install the client versions resolved by the concrete configuration adapter.
/// Runtime owns the process-wide state; no parallel copy is exposed.
pub fn set_provider_client_versions(versions: ProviderClientVersions) {
    *ACTIVE_CLIENT_VERSIONS.write() = versions;
}

pub fn codex_package_version() -> String {
    provider_client_versions().codex
}

pub fn codex_user_agent() -> String {
    crate::runtime_codex_user_agent(codex_package_version().as_str())
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

#[derive(Debug, thiserror::Error)]
/// Error fetching provider client versions.
pub enum ProviderClientVersionFetchError {
    #[error("failed to fetch latest {package} client version: {message}")]
    Fetch {
        package: &'static str,
        message: String,
    },
}

/// Fetch compatible provider CLI versions on an explicit user request.
pub async fn fetch_latest_provider_client_versions()
-> Result<ProviderClientVersions, ProviderClientVersionFetchError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CLIENT_VERSION_FETCH_TIMEOUT_SECS))
        .timeout(Duration::from_secs(CLIENT_VERSION_FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|error| ProviderClientVersionFetchError::Fetch {
            package: "npm registry",
            message: agena_failure::diagnostic::format_error_chain_with_context(
                "failed to build the provider client-version HTTP client",
                &error,
            ),
        })?;
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
    package: &'static str,
    result: Result<String, String>,
) -> Result<String, ProviderClientVersionFetchError> {
    result.map_err(|message| ProviderClientVersionFetchError::Fetch { package, message })
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
        .map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "failed to request the provider client version from the npm registry",
                &error,
            )
        })?
        .error_for_status()
        .map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "the npm registry rejected the provider client-version request",
                &error,
            )
        })?;
    let payload = response
        .json::<NpmPackageVersion>()
        .await
        .map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "failed to decode the provider client-version response from the npm registry",
                &error,
            )
        })?;
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

#[cfg(test)]
mod tests {
    use super::{
        claude_code_api_user_agent, codex_user_agent, gemini_cli_user_agent,
        gemini_node_architecture, gemini_node_platform, latest_version_result,
        valid_client_version,
    };
    use crate::RUNTIME_CODEX_ORIGINATOR;

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
        assert!(user_agent.starts_with(&format!("{RUNTIME_CODEX_ORIGINATOR}/")));
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
