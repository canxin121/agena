//! Neutral MCP configuration values parsed alongside runtime configuration.

use std::collections::BTreeMap;
use std::path::PathBuf;

use agena_plugin_host::{PluginPackage, PluginsConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MCP_PLUGIN_ID: &str = "agena.mcp";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// MCP server configuration.
pub struct McpConfig {
    pub runtime: McpRuntimeConfig,
    pub servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Runtime settings for MCP clients.
pub struct McpRuntimeConfig {
    pub token_store: McpTokenStoreConfig,
    pub reconnect: McpReconnectConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Token store settings for MCP OAuth.
pub struct McpTokenStoreConfig {
    pub enabled: bool,
}

impl Default for McpTokenStoreConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Automatic reconnect behaviour for configured but temporarily unavailable
/// MCP servers. Values are milliseconds and are bounded by the runtime before
/// constructing the connection supervisor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpReconnectConfig {
    pub enabled: bool,
    #[schemars(range(min = 1, max = 60000))]
    pub initial_delay_ms: u64,
    #[schemars(range(min = 1, max = 600000))]
    pub max_delay_ms: u64,
    #[schemars(range(min = 10, max = 60000))]
    pub poll_interval_ms: u64,
}

impl Default for McpReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            initial_delay_ms: 1_000,
            max_delay_ms: 30_000,
            poll_interval_ms: 500,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
/// Configuration of one MCP server.
pub enum McpServerConfig {
    /// Launches an MCP server as a child process and communicates over stdio.
    Stdio {
        #[serde(default = "default_enabled")]
        enabled: bool,
        #[serde(default)]
        timeouts: McpTimeoutConfig,
        process: McpStdioProcessConfig,
        #[serde(default)]
        tools: McpToolPolicyConfig,
    },
    /// Connects to a streamable HTTP MCP server.
    Http {
        #[serde(default = "default_enabled")]
        enabled: bool,
        #[serde(default)]
        timeouts: McpTimeoutConfig,
        endpoint: McpHttpEndpointConfig,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<McpHttpAuthConfig>,
        #[serde(default)]
        tools: McpToolPolicyConfig,
    },
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpTimeoutConfig {
    #[schemars(range(min = 1, max = 300000))]
    pub connect_ms: u64,
    #[schemars(range(min = 1, max = 3600000))]
    pub request_ms: u64,
}

impl Default for McpTimeoutConfig {
    fn default() -> Self {
        Self {
            connect_ms: 20_000,
            request_ms: 60_000,
        }
    }
}

impl McpServerConfig {
    pub fn enabled(&self) -> bool {
        match self {
            Self::Stdio { enabled, .. } | Self::Http { enabled, .. } => *enabled,
        }
    }

    pub fn set_enabled(&mut self, value: bool) {
        match self {
            Self::Stdio { enabled, .. } | Self::Http { enabled, .. } => *enabled = value,
        }
    }

    pub fn timeouts(&self) -> &McpTimeoutConfig {
        match self {
            Self::Stdio { timeouts, .. } | Self::Http { timeouts, .. } => timeouts,
        }
    }

    /// Validate public configuration before saving or launching a connection.
    /// Error messages identify fields without echoing credentials or arguments.
    pub fn validate(&self, name: &str) -> Result<(), String> {
        validate_mcp_server_name(name)?;
        let timeouts = self.timeouts();
        if !(1..=300_000).contains(&timeouts.connect_ms) {
            return Err("MCP connection timeout must be between 1 and 300000 ms.".into());
        }
        if !(1..=3_600_000).contains(&timeouts.request_ms) {
            return Err("MCP request timeout must be between 1 and 3600000 ms.".into());
        }
        let tools = match self {
            Self::Stdio { process, tools, .. } => {
                if process.command.trim().is_empty() || process.command.contains('\0') {
                    return Err("MCP process command must not be empty or contain NUL.".into());
                }
                if process.args.iter().any(|value| value.contains('\0')) {
                    return Err("MCP process arguments must not contain NUL.".into());
                }
                for (name, value) in &process.env {
                    validate_env_name(name)?;
                    if value.contains('\0') {
                        return Err("MCP environment values must not contain NUL.".into());
                    }
                }
                if process.cwd.as_ref().is_some_and(|path| {
                    path.as_os_str().is_empty() || path.to_string_lossy().contains('\0')
                }) {
                    return Err("MCP working directory must not be empty or contain NUL.".into());
                }
                tools
            }
            Self::Http {
                endpoint,
                auth,
                tools,
                ..
            } => {
                let url = url::Url::parse(&endpoint.url)
                    .map_err(|_| "MCP endpoint must be an absolute HTTP(S) URL.")?;
                if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                    return Err("MCP endpoint must be an absolute HTTP(S) URL.".into());
                }
                if !url.username().is_empty()
                    || url.password().is_some()
                    || url.fragment().is_some()
                {
                    return Err(
                        "MCP endpoint must not contain user information or a fragment.".into(),
                    );
                }
                validate_headers(&endpoint.headers)?;
                let authorization = endpoint
                    .headers
                    .keys()
                    .any(|key| key.eq_ignore_ascii_case("authorization"));
                if authorization && auth.is_some() {
                    return Err(
                        "Choose an authentication mode or an Authorization header, not both."
                            .into(),
                    );
                }
                match auth {
                    Some(McpHttpAuthConfig::Bearer { token }) => {
                        if token.trim().is_empty() {
                            return Err("MCP bearer token must not be empty.".into());
                        }
                        http::HeaderValue::from_str(&format!("Bearer {token}"))
                            .map_err(|_| "MCP bearer token is not a valid HTTP header value.")?;
                    }
                    Some(McpHttpAuthConfig::BearerFromEnv { env }) => validate_env_name(env)?,
                    Some(McpHttpAuthConfig::Custom { headers }) => {
                        validate_headers(headers)?;
                        if headers.keys().any(|name| {
                            endpoint
                                .headers
                                .keys()
                                .any(|existing| existing.eq_ignore_ascii_case(name))
                        }) {
                            return Err(
                                "MCP custom authentication headers duplicate endpoint headers."
                                    .into(),
                            );
                        }
                    }
                    Some(McpHttpAuthConfig::OAuth {
                        scopes,
                        client_id,
                        client_secret,
                        client_metadata_url,
                    }) => {
                        if scopes.iter().any(|scope| {
                            scope.trim().is_empty() || scope.chars().any(char::is_control)
                        }) {
                            return Err("MCP OAuth scopes must be nonempty and contain no control characters.".into());
                        }
                        if client_id.as_ref().is_some_and(|value| {
                            value.trim().is_empty() || value.chars().any(char::is_control)
                        }) {
                            return Err("MCP OAuth client ID must be nonempty and contain no control characters.".into());
                        }
                        if client_secret
                            .as_ref()
                            .is_some_and(|value| value.trim().is_empty())
                        {
                            return Err("MCP OAuth client secret must not be empty.".into());
                        }
                        if client_secret.is_some() && client_id.is_none() {
                            return Err(
                                "MCP OAuth client secret requires a pre-registered client ID."
                                    .into(),
                            );
                        }
                        if let Some(value) = client_metadata_url {
                            let metadata = url::Url::parse(value).map_err(|_| "MCP OAuth client metadata URL must be an absolute HTTPS URL with a non-root path.")?;
                            if metadata.scheme() != "https"
                                || metadata.host_str().is_none()
                                || metadata.path() == "/"
                                || !metadata.username().is_empty()
                                || metadata.password().is_some()
                                || metadata.fragment().is_some()
                            {
                                return Err("MCP OAuth client metadata URL must be an absolute HTTPS URL with a non-root path and no user information or fragment.".into());
                            }
                        }
                    }
                    None | Some(McpHttpAuthConfig::BearerFromStore) => {}
                }
                tools
            }
        };
        if tools
            .include
            .iter()
            .chain(&tools.exclude)
            .any(|pattern| pattern.trim().is_empty() || pattern.chars().any(char::is_control))
        {
            return Err(
                "MCP tool filters must be nonempty and contain no control characters.".into(),
            );
        }
        Ok(())
    }
}

pub fn validate_mcp_server_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty()
        || name != name.trim()
        || name.chars().count() > 128
        || name.chars().any(char::is_control)
    {
        return Err("MCP server name must contain 1–128 characters, without surrounding whitespace or control characters.".into());
    }
    Ok(())
}

fn validate_env_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(
            "MCP environment variable names must be nonempty and contain neither '=' nor NUL."
                .into(),
        );
    }
    Ok(())
}

fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), String> {
    let mut names = std::collections::BTreeSet::new();
    for (name, value) in headers {
        let parsed = http::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "MCP header names must be valid HTTP field names.")?;
        http::HeaderValue::from_str(value)
            .map_err(|_| "MCP header values must be valid HTTP field values.")?;
        if !names.insert(parsed.as_str().to_owned()) {
            return Err("MCP header names must be unique regardless of case.".into());
        }
    }
    Ok(())
}

/// Server-local MCP tool admission policy. It is evaluated in the connection
/// manager (initial discovery, change refresh, search and invocation), rather
/// than merely hiding a name from the model. `exclude` always wins over
/// `include`; patterns support `*` as a wildcard.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpToolPolicyConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Process settings of a stdio MCP server.
pub struct McpStdioProcessConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// HTTP endpoint settings of an MCP server.
pub struct McpHttpEndpointConfig {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
/// HTTP authentication settings of an MCP server.
pub enum McpHttpAuthConfig {
    /// Uses an inline bearer token value.
    Bearer { token: String },
    /// Reads the bearer token from an environment variable.
    BearerFromEnv { env: String },
    /// Resolves the bearer token from the configured secret store.
    BearerFromStore,
    /// OAuth tokens and registered client identities use the secret store.
    /// Optional pre-registration or client metadata supports providers that
    /// do not offer dynamic client registration.
    #[serde(alias = "oauth")]
    OAuth {
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_metadata_url: Option<String>,
    },
    /// Sends explicit custom HTTP headers.
    Custom { headers: BTreeMap<String, String> },
}

pub fn mcp_config_from_plugins(plugins: &PluginsConfig) -> Result<McpConfig, String> {
    let Some(configured_plugin) = plugins.list.get(MCP_PLUGIN_ID) else {
        return Ok(McpConfig::default());
    };
    if configured_plugin.disabled()
        || !matches!(configured_plugin.package, PluginPackage::Static { .. })
        || configured_plugin.settings().is_null()
    {
        return Ok(McpConfig::default());
    }
    serde_json::from_value(configured_plugin.settings().clone())
        .map_err(|error| format!("plugins.list.\"{MCP_PLUGIN_ID}\".settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn existing_definitions_keep_enabled_defaults_and_custom_timeouts_round_trip() {
        let mut config: McpServerConfig = serde_json::from_value(json!({
            "transport":"stdio", "process":{"command":"python3","args":["server.py"]}
        }))
        .unwrap();
        assert!(config.enabled());
        assert_eq!(config.timeouts().connect_ms, 20_000);
        assert_eq!(config.timeouts().request_ms, 60_000);
        config.validate("团队 research").unwrap();
        config.set_enabled(false);
        let value = serde_json::to_value(config).unwrap();
        assert_eq!(value["enabled"], false);
        let config: McpServerConfig = serde_json::from_value(json!({
            "transport":"http", "endpoint":{"url":"https://example.test/mcp"},
            "timeouts":{"connect_ms":1250,"request_ms":8000}, "enabled":false
        }))
        .unwrap();
        assert_eq!(config.timeouts().connect_ms, 1250);
        assert!(!config.enabled());
    }

    #[test]
    fn oauth_accepts_the_cli_alias_and_defaults_registration_fields() {
        for kind in ["oauth", "o_auth"] {
            let auth: McpHttpAuthConfig =
                serde_json::from_value(json!({"kind": kind, "scopes": ["read"]})).unwrap();
            assert!(matches!(
                &auth,
                McpHttpAuthConfig::OAuth {
                    client_id: None,
                    client_secret: None,
                    client_metadata_url: None,
                    ..
                }
            ));
            assert_eq!(serde_json::to_value(auth).unwrap()["kind"], "o_auth");
        }
    }

    #[test]
    fn invalid_connections_are_rejected_without_echoing_secret_values() {
        for value in [
            json!({"transport":"stdio","process":{"command":""}}),
            json!({"transport":"stdio","process":{"command":"python3","env":{"BAD=NAME":"private-value"}}}),
            json!({"transport":"http","endpoint":{"url":"file:///private-value"}}),
            json!({"transport":"http","endpoint":{"url":"https://user:private-value@example.test/mcp"}}),
            json!({"transport":"http","endpoint":{"url":"https://example.test/mcp","headers":{"X-Key":"private-value\nextra"}}}),
            json!({"transport":"http","endpoint":{"url":"https://example.test/mcp","headers":{"Authorization":"private-value"}},"auth":{"kind":"bearer","token":"other"}}),
            json!({"transport":"http","endpoint":{"url":"https://example.test/mcp"},"timeouts":{"connect_ms":0}}),
        ] {
            let config: McpServerConfig = serde_json::from_value(value).unwrap();
            let error = config.validate("test").unwrap_err();
            assert!(!error.contains("private-value"), "{error}");
        }
        for auth in [
            McpHttpAuthConfig::OAuth {
                scopes: vec![],
                client_id: None,
                client_secret: Some("private-value".into()),
                client_metadata_url: None,
            },
            McpHttpAuthConfig::OAuth {
                scopes: vec![],
                client_id: Some("".into()),
                client_secret: None,
                client_metadata_url: None,
            },
            McpHttpAuthConfig::OAuth {
                scopes: vec![],
                client_id: None,
                client_secret: None,
                client_metadata_url: Some("https://private-value.test/".into()),
            },
        ] {
            let config = McpServerConfig::Http {
                enabled: true,
                timeouts: Default::default(),
                endpoint: McpHttpEndpointConfig {
                    url: "https://example.test/mcp".into(),
                    headers: Default::default(),
                },
                auth: Some(auth),
                tools: Default::default(),
            };
            assert!(
                !config
                    .validate("oauth")
                    .unwrap_err()
                    .contains("private-value")
            );
        }
        assert!(validate_mcp_server_name("团队 research").is_ok());
        assert!(validate_mcp_server_name("bad\nname").is_err());
        assert!(validate_mcp_server_name(" ").is_err());
    }
}
