use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};

use crate::{McpError, McpHttpNetworkAuthorizer, McpResult};

const MAX_OUTPUT: usize = 64 * 1024;

pub(crate) fn http_auth_rejected(error: &rmcp::service::ServiceError) -> bool {
    use rmcp::transport::streamable_http_client::StreamableHttpError;

    let rmcp::service::ServiceError::TransportSend(transport) = error else {
        return false;
    };
    let mut source = Some(transport.error.as_ref() as &(dyn std::error::Error + 'static));
    while let Some(error) = source {
        if matches!(
            error.downcast_ref::<StreamableHttpError<reqwest::Error>>(),
            Some(StreamableHttpError::AuthRequired(_) | StreamableHttpError::InsufficientScope(_))
        ) || matches!(error.downcast_ref::<McpError>(), Some(McpError::Auth(_)))
        {
            return true;
        }
        source = error.source();
    }
    false
}

#[cfg(test)]
#[tokio::test]
async fn helper_needs_endpoint_and_process_authority_before_execution() {
    struct Authority {
        endpoint_allowed: bool,
        process_requested: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl McpHttpNetworkAuthorizer for Authority {
        async fn authorize(&self, target: &str) -> McpResult<()> {
            assert_eq!(target, "https://example.test/mcp");
            if self.endpoint_allowed {
                Ok(())
            } else {
                Err(McpError::PermissionDenied)
            }
        }

        async fn header_helper_policy(
            &self,
            cwd: &std::path::Path,
        ) -> McpResult<agena_process::sandbox::SandboxPolicy> {
            assert_eq!(cwd, std::path::Path::new("fixture"));
            self.process_requested
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Err(McpError::PermissionDenied)
        }
    }

    let helper = McpHeaderHelper {
        command: "unused".into(),
        cwd: PathBuf::from("fixture"),
        server_name: "fixture".into(),
        display_url: "https://example.test/mcp".into(),
    };
    for endpoint_allowed in [false, true] {
        let authority = Authority {
            endpoint_allowed,
            process_requested: Default::default(),
        };
        let mut headers = HashMap::from([("Authorization".into(), "Bearer existing".into())]);
        let result = helper
            .merge_headers(&authority, &helper.display_url, &mut headers)
            .await;
        assert!(matches!(result, Err(McpError::PermissionDenied)));
        assert_eq!(headers["Authorization"], "Bearer existing");
        assert_eq!(
            authority
                .process_requested
                .load(std::sync::atomic::Ordering::Relaxed),
            endpoint_allowed,
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpHeaderHelper {
    pub command: String,
    pub cwd: PathBuf,
    pub server_name: String,
    pub display_url: String,
}

impl McpHeaderHelper {
    pub(crate) async fn merge_headers(
        &self,
        authority: &dyn McpHttpNetworkAuthorizer,
        target: &str,
        headers: &mut HashMap<String, String>,
    ) -> McpResult<()> {
        authority.authorize(target).await?;
        let policy = authority.header_helper_policy(&self.cwd).await?;
        if policy.disabled || !agena_process::sandbox::enabled() {
            return Err(McpError::PermissionDenied);
        }
        #[cfg(windows)]
        let (shell, flags) = ("cmd.exe", &["/D", "/S", "/C"][..]);
        #[cfg(not(windows))]
        let (shell, flags) = ("sh", &["-c"][..]);
        let mut command = tokio::process::Command::new(shell);
        command
            .current_dir(&self.cwd)
            .args(flags)
            .arg(&self.command)
            .stdin(Stdio::null());
        command.env_clear();
        for name in ["PATH", "SystemRoot", "WINDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("CLAUDE_PLUGIN_ROOT", &self.cwd)
            .env("CLAUDE_CODE_MCP_SERVER_NAME", &self.server_name)
            .env("CLAUDE_CODE_MCP_SERVER_URL", &self.display_url);
        agena_process::sandbox::wrap_tokio_command(&mut command, &policy).map_err(|_| {
            McpError::Auth("could not prepare the MCP headersHelper sandbox".into())
        })?;
        let output = agena_process::output_with_optional_input_cancellable(
            command,
            &[],
            Duration::from_secs(10),
            MAX_OUTPUT,
            authority.cancellation(),
        )
        .await
        .map_err(|_| McpError::Auth("MCP headersHelper execution failed".into()))?;
        if !output.status.success() {
            return Err(McpError::Auth(
                "MCP headersHelper exited unsuccessfully".into(),
            ));
        }
        merge(headers, &output.stdout)
    }
}

fn merge(headers: &mut HashMap<String, String>, stdout: &[u8]) -> McpResult<()> {
    let output: BTreeMap<String, String> = serde_json::from_slice(stdout).map_err(|_| {
        McpError::Auth("MCP headersHelper must return a JSON object of strings".into())
    })?;
    if output.len() > 64 || stdout.len() >= MAX_OUTPUT {
        return Err(McpError::Auth(
            "MCP headersHelper output exceeds its limit".into(),
        ));
    }
    let mut validated = BTreeMap::new();
    for (name, value) in output {
        let name = name.to_ascii_lowercase();
        if http::HeaderName::from_bytes(name.as_bytes()).is_err()
            || http::HeaderValue::from_str(&value).is_err()
            || name.starts_with("sec-websocket-")
            || matches!(
                name.as_str(),
                "accept"
                    | "connection"
                    | "content-encoding"
                    | "content-length"
                    | "content-type"
                    | "host"
                    | "last-event-id"
                    | "mcp-protocol-version"
                    | "mcp-session-id"
                    | "proxy-authorization"
                    | "te"
                    | "trailer"
                    | "transfer-encoding"
                    | "upgrade"
                    | "user-agent"
            )
            || validated.insert(name, value).is_some()
        {
            return Err(McpError::Auth(
                "MCP headersHelper returned an invalid or reserved header".into(),
            ));
        }
    }
    headers.retain(|name, _| !validated.contains_key(&name.to_ascii_lowercase()));
    headers.extend(validated);
    Ok(())
}

#[cfg(test)]
#[test]
fn helper_replaces_static_headers_without_exposing_transport_controls() {
    let mut headers = HashMap::from([
        ("Authorization".into(), "Bearer stale".into()),
        ("X-Target".into(), "fixture".into()),
    ]);
    merge(
        &mut headers,
        br#"{"authorization":"Bearer fresh","X-Account":"demo"}"#,
    )
    .unwrap();
    assert_eq!(headers["authorization"], "Bearer fresh");
    assert_eq!(headers["X-Target"], "fixture");
    assert_eq!(headers["x-account"], "demo");
    assert!(!headers.contains_key("Authorization"));
    for invalid in [
        br#"{"Host":"other.example"}"#.as_slice(),
        br#"{"authorization":"a","Authorization":"b"}"#,
        br#"{"Content-Length":20}"#,
        br#"{"X-Request":"invalid\nvalue"}"#,
    ] {
        assert!(merge(&mut headers, invalid).is_err());
        assert_eq!(headers["authorization"], "Bearer fresh");
    }
}
