use std::time::Duration;

use crate::error::AppError;

const DEFAULT_PROVIDER_HTTP_TIMEOUT_SECS: u64 = 120;
const DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS: u64 = 15;

pub const CODEX_ORIGINATOR: &str = "codex_cli_rs";
pub const CODEX_MCP_CLIENT_NAME: &str = "codex-mcp-client";
pub const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.144.4";
pub const DEFAULT_CLAUDE_CLIENT_VERSION: &str = "2.1.209";
pub const DEFAULT_GEMINI_CLIENT_VERSION: &str = "0.50.0";
pub fn codex_package_version() -> String {
    DEFAULT_CODEX_CLIENT_VERSION.to_owned()
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
    format!("claude-cli/{DEFAULT_CLAUDE_CLIENT_VERSION} (external, cli)")
}

pub fn claude_code_user_agent() -> String {
    format!("claude-code/{DEFAULT_CLAUDE_CLIENT_VERSION}")
}

pub fn claude_user_web_fetch_user_agent() -> String {
    format!(
        "Claude-User ({}; +https://support.anthropic.com/)",
        claude_code_user_agent()
    )
}

pub fn gemini_cli_user_agent(model: &str) -> String {
    format!(
        "GeminiCLI/{DEFAULT_GEMINI_CLIENT_VERSION}/{model} ({}; {}; terminal)",
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
        gemini_node_architecture, gemini_node_platform,
    };

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
