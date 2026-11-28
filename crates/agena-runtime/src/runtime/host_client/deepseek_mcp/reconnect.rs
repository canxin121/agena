//! Session-owned DeepSeek MCP reconnects. Every attempt obtains fresh authority;
//! no ticket, approval, or socket policy from the originating command survives.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use agena_mcp_client::{McpError, McpHttpNetworkAuthorizer, McpManagedConnection, McpResult};
use agena_plugin_host::{
    PluginHost,
    agent_plugin::DeepSeekReconnectPolicy,
    sdk::host_api::{HostProcessResources, SESSION_SCOPED_CALL_ID},
};
use agena_process::sandbox::SandboxPolicy;
use tokio::{sync::oneshot, time::Instant};

use super::*;

pub(super) const RECONNECT_TOOL: &str = "agena.mcp.servers.reconnect";

#[derive(Clone)]
pub(super) struct ManagedAuthority {
    pub(super) sessions: Arc<crate::session::SessionManager>,
    pub(super) plugins: Arc<PluginHost>,
    pub(super) caller: PluginKey,
    pub(super) owner: PluginKey,
    pub(super) tool: String,
    pub(super) workspace: PathBuf,
    pub(super) session: i64,
    pub(super) cancellation: CancellationToken,
    pub(super) cleanup: bool,
}

impl ManagedAuthority {
    async fn executor(&self) -> McpResult<ToolExecutor> {
        if self.cancellation.is_cancelled() {
            return Err(McpError::Shutdown);
        }
        let executor = self
            .sessions
            .authorize_deepseek_mcp_manager(self.session, &self.caller, &self.tool)
            .await
            .map_err(|_| McpError::PermissionDenied)?;
        if (!self.cleanup && !Arc::ptr_eq(executor.plugin_manager(), &self.plugins))
            || executor.workspace_root() != self.workspace
            || !executor.plugin_is_enabled(&self.owner)
        {
            return Err(McpError::PermissionDenied);
        }
        Ok(executor)
    }

    async fn prepare_stdio(&self, spec: &ServerSpec) -> McpResult<McpSandboxContext> {
        let ServerSpec::Stdio { cwd, resources, .. } = spec else {
            return Err(McpError::PermissionDenied);
        };
        let prepared = self
            .sessions
            .prepare_host_process_sandbox(
                self.session,
                SESSION_SCOPED_CALL_ID,
                HostProcessResources {
                    cwd: cwd.as_ref().map(|path| path.to_string_lossy().into_owned()),
                    reads: resources.reads.clone(),
                    writes: resources.writes.clone(),
                    network: resources.network.clone(),
                    require_sandbox: true,
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| McpError::PermissionDenied)?;
        if prepared.call_bound || Path::new(&prepared.workspace_root) != self.workspace {
            return Err(McpError::PermissionDenied);
        }
        Ok(McpSandboxContext {
            scope: prepared.scope,
            workspace_root: prepared.workspace_root.into(),
            cwd: prepared.cwd.into(),
            additional_working_directories: prepared.additional_working_directories,
            policy: serde_json::from_value(prepared.policy)?,
        })
    }

    async fn connect(
        self: &Arc<Self>,
        mcp: &Arc<McpConnectionManager>,
        name: &str,
    ) -> McpResult<(Arc<McpConnectionManager>, Option<McpSandboxContext>)> {
        self.executor().await?;
        let server = mcp
            .plugin_server_in_workspace(name, &self.workspace)
            .await?;
        match &server.spec {
            ServerSpec::Http { url, .. } => {
                self.authorize(url.as_str()).await?;
                let manager = mcp
                    .for_http_scope_in_workspace(name, self.clone(), Some(&self.workspace))
                    .await?;
                self.refresh_roots(&manager).await?;
                manager.reconnect(name).await?;
                Ok((manager, None))
            }
            ServerSpec::Stdio { .. } => {
                let prepared = self.prepare_stdio(&server.spec).await?;
                let manager = tokio::select! {
                    biased;
                    _ = self.cancellation.cancelled() => return Err(McpError::Shutdown),
                    result = mcp.for_managed_sandbox(name, prepared.clone()) => result?,
                };
                Ok((manager, Some(prepared)))
            }
        }
    }

    async fn check_connection(
        &self,
        mcp: &McpConnectionManager,
        connection: &McpConnectionManager,
        name: &str,
        stdio: Option<&McpSandboxContext>,
    ) -> McpResult<()> {
        self.executor().await?;
        let server = mcp
            .plugin_server_in_workspace(name, &self.workspace)
            .await?;
        if let Some(previous) = stdio {
            if &self.prepare_stdio(&server.spec).await? != previous {
                return Err(McpError::PermissionDenied);
            }
        } else if let ServerSpec::Http { url, .. } = &server.spec {
            self.authorize(url.as_str()).await?;
            self.refresh_roots(connection).await?;
        }
        Ok(())
    }

    async fn refresh_roots(&self, connection: &McpConnectionManager) -> McpResult<()> {
        let executor = self
            .sessions
            .host_resource_callback_executor(Some(self.session), None)
            .await
            .map_err(|_| McpError::PermissionDenied)?;
        if self.cancellation.is_cancelled()
            || executor.workspace_root() != self.workspace
            || !Arc::ptr_eq(executor.plugin_manager(), &self.plugins)
        {
            return Err(McpError::PermissionDenied);
        }
        connection
            .replace_roots(
                executor
                    .sandbox_permission_snapshot()
                    .authorized_working_directories(),
            )
            .await
    }
}

#[async_trait::async_trait]
impl McpHttpNetworkAuthorizer for ManagedAuthority {
    fn cancellation(&self) -> Option<agena_process::sandbox_http::WorkerCancellation> {
        Some(self.cancellation.child_token())
    }

    fn cleanup_authority(&self) -> Option<Arc<dyn McpHttpNetworkAuthorizer>> {
        Some(Arc::new(Self {
            cancellation: CancellationToken::new(),
            cleanup: true,
            ..self.clone()
        }))
    }

    async fn authorize(&self, target: &str) -> McpResult<()> {
        let target = network_permission_target(target).map_err(|_| McpError::PermissionDenied)?;
        self.executor().await?;
        self.sessions
            .require_host_network_permission(self.session, SESSION_SCOPED_CALL_ID, target)
            .await
            .map_err(|_| McpError::PermissionDenied)
    }

    async fn authorize_resolved(&self, target: &str, address: SocketAddr) -> McpResult<bool> {
        let original = network_permission_target(target)
            .ok()
            .and_then(|target| target.parse().ok())
            .ok_or(McpError::PermissionDenied)?;
        let resolved = address
            .to_string()
            .parse()
            .map_err(|_| McpError::PermissionDenied)?;
        self.executor().await?;
        let executor = self
            .sessions
            .host_resource_callback_executor(Some(self.session), None)
            .await
            .map_err(|_| McpError::PermissionDenied)?;
        Ok(matches!(
            executor
                .sandbox_permission_snapshot()
                .authorize_resolved_network_address(&original, &resolved),
            agena_domain::PermissionDecision::Allow,
        ))
    }

    async fn sandbox_policy(&self, target: &str) -> McpResult<SandboxPolicy> {
        let target = network_permission_target(target).map_err(|_| McpError::PermissionDenied)?;
        self.executor().await?;
        let prepared = self
            .sessions
            .prepare_host_worker_sandbox(
                self.session,
                SESSION_SCOPED_CALL_ID,
                HostProcessResources {
                    network: vec![target],
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| McpError::PermissionDenied)?;
        if prepared.call_bound {
            return Err(McpError::PermissionDenied);
        }
        Ok(serde_json::from_value(prepared.policy)?)
    }
}

pub(super) struct ManagedRestart {
    pub(super) authority: Arc<ManagedAuthority>,
    pub(super) mcp: Arc<McpConnectionManager>,
    pub(super) name: String,
    pub(super) scope: Arc<McpManagedConnection>,
    pub(super) previous: Option<Arc<McpManagedConnection>>,
    pub(super) policy: DeepSeekReconnectPolicy,
}

impl ManagedRestart {
    pub(super) fn start(self) -> oneshot::Receiver<McpResult<()>> {
        let (first, started) = oneshot::channel();
        tokio::spawn(async move {
            let confirmed = self.supervise(first).await;
            self.scope.finish(confirmed);
        });
        started
    }

    async fn supervise(&self, first: oneshot::Sender<McpResult<()>>) -> bool {
        if let Some(previous) = &self.previous
            && !previous.wait_closed().await
        {
            self.cleanup_failed();
            let _ = first.send(Err(McpError::Transport(
                "previous MCP connection did not close".into(),
            )));
            return false;
        }
        let mut first = Some(first);
        let mut attempts = 0_u64;
        let cancel = self.scope.cancellation();
        while !cancel.is_cancelled() {
            self.scope
                .publish(json!({ "state": "connecting", "attempt": attempts }));
            // The HTTP worker observes cancellation through its authority.
            // Let negotiation report whether it already opened a remote session.
            let connection = self.authority.connect(&self.mcp, &self.name).await;
            let last_error = match connection {
                Ok((connection, stdio)) => {
                    let connected_at = Instant::now();
                    self.scope.set_transport(Some(connection.clone()));
                    self.scope.publish(connection_status(
                        connection.connection_observation(&self.name).await,
                    ));
                    if let Some(first) = first.take() {
                        let _ = first.send(Ok(()));
                    }
                    let result = self.wait_for_loss(&connection, stdio.as_ref()).await;
                    self.scope.set_transport(None);
                    if !connection.close_managed_transport(&self.name).await {
                        self.cleanup_failed();
                        return false;
                    }
                    if cancel.is_cancelled() {
                        break;
                    }
                    if connected_at.elapsed() >= self.policy.max_delay {
                        attempts = 0;
                    }
                    result
                }
                Err(McpError::UnconfirmedClosure) => {
                    self.cleanup_failed();
                    if let Some(first) = first.take() {
                        let _ = first.send(Err(McpError::UnconfirmedClosure));
                    }
                    return false;
                }
                Err(error) => error,
            };
            let message = last_error.user_message();
            if matches!(last_error, McpError::PermissionDenied | McpError::Shutdown)
                || !self.policy.enabled
            {
                self.failed("reconnect-disabled", &message);
                if let Some(first) = first.take() {
                    let _ = first.send(Err(last_error));
                }
                break;
            }
            attempts = attempts.saturating_add(1);
            if attempts > self.policy.max_attempts {
                self.failed("gave-up", &message);
                break;
            }
            let factor = 1_u32 << attempts.saturating_sub(1).min(31);
            let delay = self
                .policy
                .initial_delay
                .saturating_mul(factor)
                .min(self.policy.max_delay);
            let next_retry_at = chrono::Utc::now()
                .timestamp_millis()
                .saturating_add(i64::try_from(delay.as_millis()).unwrap_or(i64::MAX));
            self.scope.publish(json!({
                "state": "reconnecting", "attempt": attempts, "maxAttempts": self.policy.max_attempts,
                "nextRetryAtMs": next_retry_at, "lastError": message,
            }));
            if let Some(first) = first.take() {
                let _ = first.send(Ok(()));
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(delay) => {}
            }
        }
        true
    }

    async fn wait_for_loss(
        &self,
        connection: &McpConnectionManager,
        stdio: Option<&McpSandboxContext>,
    ) -> McpError {
        let cancel = self.scope.cancellation();
        let mut observations = connection.observe_plugin_connections();
        let session_id = match self.authority.session.try_into() {
            Ok(session) => session,
            Err(_) => return McpError::PermissionDenied,
        };
        let subscribe = || {
            self.authority
                .sessions
                .kernel()
                .reader
                .watch(session_id, None)
        };
        let Ok(mut changes) = subscribe() else {
            return McpError::PermissionDenied;
        };
        // The policy may have changed during negotiation. Subscribe before
        // rechecking so subsequent changes cannot fall between the two.
        let verify = || async {
            let _permit = BACKGROUND_POLICY_CHECKS
                .acquire()
                .await
                .map_err(|_| McpError::Shutdown)?;
            self.authority
                .check_connection(&self.mcp, connection, &self.name, stdio)
                .await
        };
        let check = tokio::select! {
            biased;
            _ = cancel.cancelled() => return McpError::Shutdown,
            result = verify() => result,
        };
        if let Err(error) = check {
            return error;
        }
        loop {
            let _ = observations.borrow_and_update();
            let observation = connection.connection_observation(&self.name).await;
            if !matches!(observation, McpConnectionObservation::Connected { .. }) {
                return McpError::TransportClosed;
            }
            self.scope.publish(connection_status(observation));
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return McpError::Shutdown,
                changed = observations.changed() => {
                    if changed.is_err() { return McpError::Shutdown; }
                }
                change = next_catalog_change(&mut changes) => {
                    match change {
                        Ok(change) if session_was_deleted(&change) => return McpError::Shutdown,
                        Err(WatchError::Reset { .. }) => match subscribe() {
                            Ok(watch) => changes = watch,
                            Err(_) => return McpError::PermissionDenied,
                        },
                        Err(WatchError::Closed) => return McpError::Shutdown,
                        Ok(_) => {}
                    }
                    let check = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return McpError::Shutdown,
                        result = verify() => result,
                    };
                    if let Err(error) = check { return error; }
                }
            }
        }
    }

    fn failed(&self, reason: &str, message: &str) {
        self.scope
            .publish(json!({ "state": "failed", "reason": reason, "lastError": message }));
    }

    fn cleanup_failed(&self) {
        self.scope.publish(json!({
            "state": "failed", "reason": "cleanup", "result": { "closeConfirmed": false },
        }));
    }
}
