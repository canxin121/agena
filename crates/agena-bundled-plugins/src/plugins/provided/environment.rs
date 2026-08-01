//! Readiness waiting for managed development environments and services.

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const ENVIRONMENT_PLUGIN_ID: &str = "agena.environment";

pub(crate) struct EnvironmentPlugin {
    workspace_root: OnceLock<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WaitCondition {
    Path {
        path: String,
    },
    Tcp {
        host: String,
        port: u16,
    },
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_status: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contains: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    minimum("timeout_ms", 1),
    maximum("timeout_ms", 600000),
    minimum("interval_ms", 50),
    maximum("interval_ms", 30000)
)]
#[serde(deny_unknown_fields)]
struct EnvironmentWaitInput {
    condition: WaitCondition,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_interval_ms")]
    interval_ms: u64,
}

const fn default_timeout_ms() -> u64 {
    60_000
}

const fn default_interval_ms() -> u64 {
    500
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "environment",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Wait for filesystem, TCP, or HTTP environment readiness.",
    display = detailed
)]
impl EnvironmentPlugin {
    pub(crate) fn new() -> Self {
        Self {
            workspace_root: OnceLock::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, _host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.workspace_root.set(ctx.workspace_root).map_err(|_| {
            PluginError::internal("environment workspace initialized more than once")
        })?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        summary = "Wait until a path, TCP endpoint, or HTTP health check is ready.",
        read_only,
        filesystem_read,
        network,
        display = detailed,
        concurrency_safe
    )]
    async fn wait(&self, input: &EnvironmentWaitInput) -> SdkResult<ToolInvokeOutput> {
        self.validate_condition(&input.condition).await?;
        let started = tokio::time::Instant::now();
        let deadline = started + Duration::from_millis(input.timeout_ms);
        let mut attempts = 0u64;
        loop {
            attempts = attempts.saturating_add(1);
            let last_error = match self.check_condition(&input.condition).await {
                Ok(summary) => {
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    return Ok(ToolInvokeOutput::from_parts(
                        "environment ready",
                        format!("Ready after {elapsed_ms} ms · {summary}"),
                        format!("Environment became ready after {elapsed_ms} ms: {summary}"),
                        Some(serde_json::json!({
                            "ready": true,
                            "attempts": attempts,
                            "elapsed_ms": elapsed_ms,
                            "summary": summary,
                        })),
                        std::collections::BTreeMap::from([
                            ("ready".to_string(), "true".to_string()),
                            ("attempts".to_string(), attempts.to_string()),
                        ]),
                        Vec::new(),
                    ));
                }
                Err(error) => error,
            };
            let now = tokio::time::Instant::now();
            if now >= deadline {
                let diagnostic = format!(
                    "environment readiness timed out after {} ms and {attempts} attempt(s): {last_error}",
                    input.timeout_ms
                );
                let public_detail = self.timeout_public_detail(
                    &input.condition,
                    input.timeout_ms,
                    attempts,
                    last_error.as_str(),
                );
                return Err(PluginError::timeout_with_public_detail(
                    diagnostic,
                    public_detail,
                ));
            }
            tokio::time::sleep(
                Duration::from_millis(input.interval_ms)
                    .min(deadline.saturating_duration_since(now)),
            )
            .await;
        }
    }

    async fn validate_condition(&self, condition: &WaitCondition) -> SdkResult<()> {
        match condition {
            WaitCondition::Path { path } => {
                if path.trim().is_empty() {
                    return Err(PluginError::invalid_params("path must not be empty"));
                }
                Ok(())
            }
            WaitCondition::Tcp { host: target, port } => {
                if target.trim().is_empty() || *port == 0 {
                    return Err(PluginError::invalid_params(
                        "tcp host must be non-empty and port must be greater than zero",
                    ));
                }
                validate_public_host(target, *port).await
            }
            WaitCondition::Http { url, .. } => {
                let url = parse_http_url(url)?;
                let target = url
                    .host_str()
                    .ok_or_else(|| PluginError::invalid_params("HTTP URL has no host"))?;
                let port = url
                    .port_or_known_default()
                    .ok_or_else(|| PluginError::invalid_params("HTTP URL has no port"))?;
                validate_public_host(target, port).await
            }
        }
    }

    async fn check_condition(&self, condition: &WaitCondition) -> Result<String, String> {
        match condition {
            WaitCondition::Path { path } => {
                let root = self
                    .workspace_root
                    .get()
                    .ok_or_else(|| "environment plugin invoked before init".to_string())?;
                let target = resolve_path(root, path);
                target
                    .exists()
                    .then(|| format!("path '{}' exists", target.display()))
                    .ok_or_else(|| format!("path '{}' does not exist", target.display()))
            }
            WaitCondition::Tcp { host, port } => tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::TcpStream::connect((host.as_str(), *port)),
            )
            .await
            .map_err(|_| format!("TCP connection to {host}:{port} timed out"))?
            .map(|_| format!("TCP {host}:{port} accepts connections"))
            .map_err(|error| format!("TCP {host}:{port} is not ready: {error}")),
            WaitCondition::Http {
                url,
                expected_status,
                contains,
            } => {
                let client = reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(10))
                    .build()
                    .map_err(|error| format!("cannot build HTTP client: {error}"))?;
                let mut response = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|error| format!("HTTP request failed: {error}"))?;
                let status = response.status().as_u16();
                let expected = expected_status.unwrap_or(200);
                if status != expected {
                    return Err(format!("HTTP status is {status}, expected {expected}"));
                }
                if let Some(needle) = contains.as_deref() {
                    let mut body = Vec::new();
                    while let Some(chunk) = response
                        .chunk()
                        .await
                        .map_err(|error| format!("cannot read HTTP response: {error}"))?
                    {
                        if body.len().saturating_add(chunk.len()) > 1024 * 1024 {
                            return Err("HTTP readiness body exceeds 1 MiB".to_string());
                        }
                        body.extend_from_slice(&chunk);
                    }
                    let body = String::from_utf8_lossy(&body);
                    if !body.contains(needle) {
                        return Err(format!("HTTP body does not contain {needle:?}"));
                    }
                }
                Ok(format!("HTTP {url} returned {status}"))
            }
        }
    }

    fn timeout_public_detail(
        &self,
        condition: &WaitCondition,
        timeout_ms: u64,
        attempts: u64,
        last_error: &str,
    ) -> String {
        let condition_detail = match condition {
            WaitCondition::Path { path } => {
                let root = self.workspace_root.get();
                let target =
                    root.map_or_else(|| PathBuf::from(path), |root| resolve_path(root, path));
                let visible = root
                    .and_then(|root| target.strip_prefix(root).ok())
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .map(|relative| relative.display().to_string())
                    .or_else(|| {
                        target
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                    })
                    .unwrap_or_else(|| path.clone());
                format!("path '{visible}' does not exist")
            }
            WaitCondition::Tcp { host, port } => {
                format!("TCP {host}:{port} is not ready: {last_error}")
            }
            WaitCondition::Http { url, .. } => {
                format!("HTTP endpoint {url} is not ready: {last_error}")
            }
        };
        format!(
            "Environment readiness timed out after {timeout_ms} ms and {attempts} attempt(s): {condition_detail}."
        )
    }
}

async fn validate_public_host(target: &str, port: u16) -> SdkResult<()> {
    let addresses = tokio::net::lookup_host((target, port))
        .await
        .map_err(|error| PluginError::internal(format!("failed to resolve {target}: {error}")))?
        .map(|address| address.ip())
        .collect::<BTreeSet<_>>();
    if addresses.is_empty() {
        return Err(PluginError::internal(format!(
            "DNS resolution returned no addresses for {target}"
        )));
    }
    for address in addresses {
        if !is_public_address(address) {
            return Err(PluginError::invalid_params(format!(
                "environment target `{target}` resolves to non-public address `{address}`"
            )));
        }
    }
    Ok(())
}

fn parse_http_url(value: &str) -> SdkResult<url::Url> {
    let url = url::Url::parse(value.trim())
        .map_err(|error| PluginError::invalid_params(format!("invalid HTTP URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(PluginError::invalid_params(
            "environment HTTP wait requires an absolute http/https URL",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PluginError::invalid_params(
            "environment HTTP wait URL cannot contain credentials",
        ));
    }
    Ok(url)
}

fn resolve_path(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_unspecified())
        }
        IpAddr::V6(address) => {
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use agena_plugin_host::PluginError;
    use agena_plugin_host::sdk::Plugin;

    use super::{EnvironmentPlugin, WaitCondition, parse_http_url};

    #[test]
    fn manifest_exposes_environment_wait() {
        let manifest = EnvironmentPlugin::new().manifest();
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "wait");
    }

    #[test]
    fn http_wait_rejects_credentials_and_non_http_urls() {
        assert!(parse_http_url("file:///tmp/ready").is_err());
        assert!(parse_http_url("https://user:secret@example.com/health").is_err());
        assert!(parse_http_url("https://example.com/health").is_ok());
    }

    #[test]
    fn readiness_timeout_public_detail_keeps_actionable_relative_path() {
        let plugin = EnvironmentPlugin::new();
        plugin
            .workspace_root
            .set(PathBuf::from("/Users/private/workspace"))
            .expect("set workspace root");
        let detail = plugin.timeout_public_detail(
            &WaitCondition::Path {
                path: "tmp/delayed.txt".to_owned(),
            },
            15_000,
            31,
            "path '/Users/private/workspace/tmp/delayed.txt' does not exist",
        );
        assert_eq!(
            detail,
            "Environment readiness timed out after 15000 ms and 31 attempt(s): path 'tmp/delayed.txt' does not exist."
        );
        let error = PluginError::timeout_with_public_detail("private diagnostic", detail);
        assert!(error.failure.user.fallback.contains("tmp/delayed.txt"));
        assert!(!error.failure.user.fallback.contains("/Users/private"));
    }
}
