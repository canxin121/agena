use super::*;
use serde::Serialize;

pub struct McpProbeOptions {
    pub timeout: Duration,
    pub close_timeout: Duration,
    pub max_pages: usize,
    pub max_tool_names: usize,
    pub cancellation: CancellationToken,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ProbeOutcome {
    Connected {
        ok: bool,
        #[serde(rename = "serverInfo", skip_serializing_if = "Option::is_none")]
        server_info: Option<ProbeIdentity>,
        #[serde(rename = "toolNames")]
        tool_names: Vec<String>,
        truncated: bool,
    },
    Failed {
        ok: bool,
        stage: &'static str,
        message: String,
    },
}

#[derive(Serialize)]
struct ProbeIdentity {
    name: String,
    version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpProbeResult {
    #[serde(flatten)]
    outcome: ProbeOutcome,
    duration_ms: u64,
    close_confirmed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cleanup_error: Option<&'static str>,
}

impl McpConnectionManager {
    /// Negotiate and list raw tool names through an independent authorized transport.
    pub async fn probe_plugin_server(
        &self,
        name: &str,
        server: ScopedPluginServer,
        authorizer: Arc<dyn crate::McpHttpNetworkAuthorizer>,
        options: McpProbeOptions,
    ) -> McpResult<McpProbeResult> {
        if options.cancellation.is_cancelled() {
            return Ok(McpProbeResult {
                outcome: ProbeOutcome::Failed {
                    ok: false,
                    stage: "connect",
                    message: "probe aborted".into(),
                },
                duration_ms: 0,
                close_confirmed: true,
                cleanup_error: None,
            });
        }
        let ScopedPluginServer { spec, redactions } = server;
        if matches!(&spec, ServerSpec::Stdio { sandbox, .. } if
            sandbox.as_ref().is_none_or(|policy| policy.disabled) || !agena_process::sandbox::enabled())
        {
            return Err(scoped::missing_scope(name));
        }
        let mut probe = Self::new(&self.client_name, &self.client_version)
            .with_http_network_authorizer(authorizer);
        probe.token_store = self.token_store.clone();
        let events = Arc::new(ServerEventState::default());
        let mut handler = probe.client_handler(
            name,
            events.clone(),
            spec.tool_policy().clone(),
            options.timeout,
            redactions.clone(),
        );
        handler.info.capabilities = ClientCapabilities::default();
        let start = tokio::time::Instant::now();
        let deadline = start + options.timeout;
        let elapsed_ms = || u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let failed = |stage, message| ProbeOutcome::Failed {
            ok: false,
            stage,
            message,
        };
        let aborted = || "probe aborted".to_owned();
        let timed_out = || "MCP probe timed out".to_owned();
        let negotiation = probe.open_transport(name, spec, handler, true);
        let (mut running, _) = match tokio::select! {
            biased;
            _ = options.cancellation.cancelled() => Err((aborted(), false)),
            result = tokio::time::timeout_at(deadline, negotiation) => match result {
                Ok(Ok(transport)) => Ok(transport),
                Ok(Err(error)) => Err((error.user_message(), !events.http_session_open.load(Ordering::Acquire))),
                Err(_) => Err((timed_out(), false)),
            },
        } {
            Ok(connected) => connected,
            Err((message, close_confirmed)) => {
                return Ok(McpProbeResult {
                    outcome: failed("connect", message),
                    duration_ms: elapsed_ms(),
                    close_confirmed,
                    cleanup_error: (!close_confirmed)
                        .then_some("MCP negotiation cleanup did not confirm closure"),
                });
            }
        };

        let peer = running.peer();
        let info = peer.peer_info();
        let server_info = info
            .as_ref()
            .and_then(|info| info.server_info.as_ref())
            .map(|info| ProbeIdentity {
                name: redactions.text(info.name.clone()),
                version: redactions.text(info.version.clone()),
            });
        let list = async {
            if info.is_some_and(|info| info.capabilities.tools.is_some()) {
                list_tool_names(peer, &redactions, options.max_pages, options.max_tool_names).await
            } else {
                Ok((Vec::new(), false))
            }
        };
        let outcome = tokio::select! {
            biased;
            _ = options.cancellation.cancelled() => failed("list", aborted()),
            result = tokio::time::timeout_at(deadline, list) => match result {
                Ok(Ok((tool_names, truncated))) => ProbeOutcome::Connected {
                    ok: true, server_info, tool_names, truncated,
                },
                Ok(Err(error)) => failed("list", error.user_message()),
                Err(_) => failed("list", timed_out()),
            },
        };
        let duration_ms = elapsed_ms();
        let cleanup_error = match running.close_with_timeout(options.close_timeout).await {
            Ok(Some(rmcp::service::QuitReason::Cancelled | rmcp::service::QuitReason::Closed)) => {
                events
                    .http_session_open
                    .load(Ordering::Acquire)
                    .then_some("MCP probe did not confirm remote session closure")
            }
            Ok(None) => Some("MCP probe cleanup timed out"),
            _ => Some("MCP probe cleanup failed"),
        };
        Ok(McpProbeResult {
            outcome,
            duration_ms,
            close_confirmed: cleanup_error.is_none(),
            cleanup_error,
        })
    }
}

async fn list_tool_names(
    peer: &Peer<RoleClient>,
    redactions: &McpRedactions,
    max_pages: usize,
    max_tool_names: usize,
) -> McpResult<(Vec<String>, bool)> {
    let mut cursor = None;
    let mut names = Vec::new();
    for _ in 0..max_pages {
        let page = peer.list_tools(pagination_params(cursor)).await?;
        for tool in page.tools {
            if names.len() == max_tool_names {
                return Ok((names, true));
            }
            names.push(redactions.text(tool.name.into_owned()));
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok((names, false));
        }
    }
    Ok((names, true))
}
