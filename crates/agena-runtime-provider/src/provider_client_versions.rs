//! Explicit Runtime-owned provider-client version refresh.

use std::time::Duration;

use agena_provider::ProviderClientVersions;
use serde::Deserialize;

const CLIENT_VERSION_FETCH_TIMEOUT_SECS: u64 = 5;
const CODEX_VERSION_URL: &str = "https://registry.npmjs.org/@openai%2Fcodex/latest";
const CLAUDE_VERSION_URL: &str = "https://registry.npmjs.org/@anthropic-ai%2Fclaude-code/latest";
const GEMINI_VERSION_URL: &str = "https://registry.npmjs.org/@google%2Fgemini-cli/latest";

/// Stable client identity used by Runtime-owned MCP and provider transports.
pub const RUNTIME_CODEX_MCP_CLIENT_NAME: &str = "codex-mcp-client";

/// Immutable client identity owned by a runtime generation or standalone
/// provider. Candidate construction never changes another owner's requests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderClientIdentity {
    versions: ProviderClientVersions,
}

impl ProviderClientIdentity {
    pub fn new(versions: ProviderClientVersions) -> Self {
        Self { versions }
    }

    pub fn versions(&self) -> ProviderClientVersions {
        self.versions.clone()
    }

    pub fn codex_package_version(&self) -> String {
        self.versions.codex.clone()
    }

    pub fn codex_user_agent(&self) -> String {
        crate::runtime_codex_user_agent(&self.versions.codex)
    }

    pub fn claude_code_api_user_agent(&self) -> String {
        format!("claude-cli/{} (external, cli)", self.versions.claude)
    }

    pub fn claude_code_user_agent(&self) -> String {
        format!("claude-code/{}", self.versions.claude)
    }

    pub fn claude_user_web_fetch_user_agent(&self) -> String {
        format!(
            "Claude-User ({}; +https://support.anthropic.com/)",
            self.claude_code_user_agent()
        )
    }

    pub fn gemini_cli_user_agent(&self, model: &str) -> String {
        format!(
            "GeminiCLI/{}/{model} ({}; {}; terminal)",
            self.versions.gemini,
            gemini_node_platform(),
            gemini_node_architecture()
        )
    }
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

/// Fetch the current compatible provider CLI versions from npm.
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
