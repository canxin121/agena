use super::*;
use agena_process::sandbox::SandboxPolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StdioResourceDeclarations {
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub network: Vec<String>,
}

/// Created by the runtime after resolving its live principal and resource
/// approvals. This type is not an MCP protocol input or plugin RPC argument.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct McpSandboxContext {
    pub scope: String,
    pub workspace_root: PathBuf,
    pub cwd: PathBuf,
    pub additional_working_directories: Vec<PathBuf>,
    pub policy: SandboxPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpConnectionObservation {
    Closed,
    Connecting,
    Connected {
        tool_count: usize,
        connected_at_ms: i64,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ScopedServerKey {
    server: String,
    revision: u64,
    context: McpSandboxContext,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PluginHttpCallKey {
    server: String,
    session: i64,
    origin: String,
}

pub(super) fn missing_scope(server: &str) -> McpError {
    McpError::Transport(format!(
        "MCP server `{server}` requires a host-authorized execution scope"
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SandboxConnection<'a> {
    Reuse,
    Restart,
    Managed,
    Channel(&'a ServerSpec),
}

struct AppStdioPeer {
    key: ScopedServerKey,
    manager: Arc<McpConnectionManager>,
    stored: StoredServerSpec,
    peer: Arc<ConnectedServer>,
}

impl ServerSpec {
    fn bind_stdio_sandbox(&mut self, context: &McpSandboxContext) -> McpResult<()> {
        let Self::Stdio { cwd, sandbox, .. } = self else {
            return Err(McpError::Transport(
                "MCP HTTP servers require a call-owned authorized HTTP scope".into(),
            ));
        };
        if sandbox.is_some() {
            return Err(McpError::Transport(
                "an MCP process cannot expand or rebind its existing sandbox scope".into(),
            ));
        }
        *cwd = Some(context.cwd.clone());
        *sandbox = Some(context.policy.clone());
        Ok(())
    }
}

impl McpConnectionManager {
    /// Make a disconnected, call-owned HTTP manager. The caller registers its
    /// lifetime cleanup before connecting, including failure and cancellation.
    /// No live connection or callback authority is taken from the shared manager.
    pub async fn for_http_scope(
        &self,
        name: &str,
        authorizer: Arc<dyn crate::McpHttpNetworkAuthorizer>,
    ) -> McpResult<Arc<Self>> {
        self.for_http_scope_in_workspace(name, authorizer, None)
            .await
    }

    pub async fn for_http_scope_in_workspace(
        &self,
        name: &str,
        authorizer: Arc<dyn crate::McpHttpNetworkAuthorizer>,
        workspace: Option<&Path>,
    ) -> McpResult<Arc<Self>> {
        let _lifecycle = self.lifecycle.read().await;
        self.new_http_scope(name, authorizer, workspace, true).await
    }

    /// An App peer can outlive its initiating call. The initial negotiation
    /// uses that call, followed by explicitly authorized guest operations.
    pub async fn for_app_http_scope_in_workspace(
        &self,
        name: &str,
        authorizer: Arc<dyn crate::McpHttpNetworkAuthorizer>,
        workspace: Option<&Path>,
    ) -> McpResult<Arc<Self>> {
        let _lifecycle = self.lifecycle.read().await;
        self.new_http_scope(name, authorizer, workspace, false)
            .await
    }

    async fn new_http_scope(
        &self,
        name: &str,
        authorizer: Arc<dyn crate::McpHttpNetworkAuthorizer>,
        workspace: Option<&Path>,
        background_enabled: bool,
    ) -> McpResult<Arc<Self>> {
        let mut stored = self
            .inner
            .read()
            .await
            .specs
            .get(name)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))?;
        if let Some(workspace) = workspace {
            let resolved = stored.in_workspace(workspace).await?;
            stored.spec = resolved.spec;
            stored.redactions = resolved.redactions;
        }
        stored.resolve_workspace = None;
        if !matches!(&stored.spec, ServerSpec::Http { .. }) {
            return Err(McpError::Transport(
                "MCP HTTP scope requires an HTTP server".into(),
            ));
        }
        let mut manager = Self::new(&self.client_name, &self.client_version)
            .with_apps(self.apps_enabled)
            .with_timeouts(stored.connect_timeout, stored.request_timeout)
            .with_http_network_authorizer(authorizer);
        manager.token_store = self.token_store.clone();
        manager.http_background_enabled = background_enabled;
        manager.observations = self.observations.clone();
        if stored.plugin_scoped {
            manager = manager.with_roots(
                workspace
                    .filter(|path| path.is_absolute())
                    .map(Path::to_owned),
            );
        } else {
            manager.roots = Arc::new(RwLock::new(self.roots.read().await.clone()));
        }
        manager
            .inner
            .write()
            .await
            .specs
            .insert(name.to_owned(), stored);
        Ok(Arc::new(manager))
    }

    /// Keep the manager for repeated operations in the originating call.
    /// A subsequent call must supply fresh connection authority.
    pub async fn for_plugin_http_call(
        self: &Arc<Self>,
        name: &str,
        authorizer: Arc<dyn crate::McpHttpNetworkAuthorizer>,
        workspace: &Path,
        session: i64,
        call: &str,
        finished: CancellationToken,
    ) -> McpResult<Arc<Self>> {
        let _lifecycle = self.lifecycle.read().await;
        let operation = self.server_operation_lock(name).await;
        let _operation = operation.lock().await;
        if finished.is_cancelled() || !self.is_plugin_server(name).await {
            return Err(missing_scope(name));
        }
        let key = PluginHttpCallKey {
            server: name.to_owned(),
            session,
            origin: call.to_owned(),
        };
        if let Some(manager) = self.plugin_http_calls.lock().await.get(&key).cloned() {
            return Ok(manager);
        }
        let manager = self
            .new_http_scope(name, authorizer, Some(workspace), true)
            .await?;
        {
            let mut calls = self.plugin_http_calls.lock().await;
            if calls.len() >= 128 {
                return Err(McpError::Transport(
                    "MCP HTTP call scope limit reached".into(),
                ));
            }
            calls.insert(key.clone(), Arc::clone(&manager));
        }
        let owner = Arc::downgrade(self);
        tokio::spawn(async move {
            finished.cancelled().await;
            if let Some(owner) = owner.upgrade() {
                let _lifecycle = owner.lifecycle.read().await;
                let retired = owner.plugin_http_calls.lock().await.remove(&key);
                if let Some(retired) = retired {
                    owner.notify_observation();
                    retired.shutdown_local().await;
                }
            }
        });
        Ok(manager)
    }

    pub fn observe_plugin_connections(&self) -> watch::Receiver<u64> {
        self.observations.subscribe()
    }

    pub fn connection_revision(&self) -> u64 {
        *self.observations.borrow()
    }

    pub(super) fn notify_observation(&self) {
        self.observations.send_modify(|revision| *revision += 1);
    }

    pub async fn connection_observation(&self, name: &str) -> McpConnectionObservation {
        let (server, failure, connecting) = {
            let inner = self.inner.read().await;
            (
                inner.servers.get(name).cloned(),
                inner.last_failures.get(name).cloned(),
                inner
                    .connecting
                    .get(name)
                    .is_some_and(|active| active.strong_count() > 0),
            )
        };
        match (server, failure) {
            (Some(server), _) if !server.peer.is_transport_closed() => {
                McpConnectionObservation::Connected {
                    tool_count: server.events.tools.read().await.len(),
                    connected_at_ms: server.connected_at_ms,
                }
            }
            (_, Some(failure)) => McpConnectionObservation::Failed {
                message: failure.user.fallback,
            },
            _ if connecting => McpConnectionObservation::Connecting,
            _ => McpConnectionObservation::Closed,
        }
    }

    /// Observe live connections in the session without resolving credentials,
    /// running network requests, or preparing a sandbox.
    pub async fn plugin_connection_observation(
        &self,
        name: &str,
        workspace: &Path,
        session: i64,
        call: &str,
    ) -> McpConnectionObservation {
        let key = PluginHttpCallKey {
            server: name.to_owned(),
            session,
            origin: call.to_owned(),
        };
        let http = self.plugin_http_calls.lock().await.get(&key).cloned();
        if let Some(http) = http {
            return http.connection_observation(name).await;
        }
        let session_scope = format!("session:{session}");
        let call_prefix = format!("{session_scope}:call:");
        let call_suffix = format!(":origin:{call}");
        let stdio = {
            let scopes = self.scoped_servers.lock().await;
            let mut stdio = None;
            for (key, manager) in scopes.iter() {
                let scope = &key.context.scope;
                if key.server != name || key.context.workspace_root != workspace {
                    continue;
                }
                if scope.starts_with(&call_prefix) && scope.ends_with(&call_suffix) {
                    stdio = Some(Arc::clone(manager));
                    break;
                }
                if scope == &session_scope {
                    stdio = Some(Arc::clone(manager));
                }
            }
            stdio
        };
        match stdio {
            Some(stdio) => stdio.connection_observation(name).await,
            None => McpConnectionObservation::Closed,
        }
    }

    pub async fn server_spec(&self, name: &str) -> McpResult<ServerSpec> {
        self.inner
            .read()
            .await
            .specs
            .get(name)
            .map(|stored| stored.spec.clone())
            .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))
    }

    pub async fn server_spec_in_workspace(
        &self,
        name: &str,
        workspace: &Path,
    ) -> McpResult<ServerSpec> {
        let stored = self
            .inner
            .read()
            .await
            .specs
            .get(name)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))?;
        stored
            .in_workspace(workspace)
            .await
            .map(|resolved| resolved.spec)
    }

    pub async fn plugin_server_in_workspace(
        &self,
        name: &str,
        workspace: &Path,
    ) -> McpResult<ScopedPluginServer> {
        let stored = self
            .inner
            .read()
            .await
            .specs
            .get(name)
            .filter(|stored| stored.plugin_scoped)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))?;
        stored.in_workspace(workspace).await
    }

    /// Select a process by configuration revision, owning scope, working
    /// directory and the entire kernel policy. A narrower caller never finds
    /// a shared server-name-only process.
    pub async fn for_sandbox_scope(
        self: &Arc<Self>,
        name: &str,
        context: McpSandboxContext,
    ) -> McpResult<Arc<Self>> {
        self.sandbox_scope(name, context, SandboxConnection::Reuse)
            .await
            .map(|(manager, _)| manager)
    }

    /// Return the configured identity for the exact session-scoped process
    /// that rendered an App. Call-owned processes expire with their approval.
    pub async fn app_stdio_spec(
        &self,
        session: i64,
        name: &str,
        workspace: &Path,
        peer_id: Uuid,
    ) -> McpResult<ServerSpec> {
        let _lifecycle = self.lifecycle.read().await;
        let operation = self.server_operation_lock(name).await;
        let _operation = operation.lock().await;
        let live = self
            .app_stdio_peer(session, name, workspace, peer_id)
            .await?;
        let original = self.server_spec_in_workspace(name, workspace).await?;
        let mut authorized = original.clone();
        authorized.bind_stdio_sandbox(&live.key.context)?;
        if authorized != live.stored.spec {
            return Err(McpError::PermissionDenied);
        }
        Ok(original)
    }

    /// Borrow the rendering peer under an identical, freshly evaluated
    /// process policy. This operation never starts or reconnects a process.
    pub async fn borrow_app_stdio(
        &self,
        session: i64,
        name: &str,
        workspace: &Path,
        peer_id: Uuid,
        mut source: ServerSpec,
        context: &McpSandboxContext,
    ) -> McpResult<Arc<Self>> {
        let _lifecycle = self.lifecycle.read().await;
        let operation = self.server_operation_lock(name).await;
        let _operation = operation.lock().await;
        let live = self
            .app_stdio_peer(session, name, workspace, peer_id)
            .await?;
        if &live.key.context != context {
            return Err(McpError::PermissionDenied);
        }
        source.bind_stdio_sandbox(context)?;
        if source != live.stored.spec {
            return Err(McpError::PermissionDenied);
        }
        let mut view =
            Self::new(&self.client_name, &self.client_version).with_apps(self.apps_enabled);
        view.roots = live.manager.roots.clone();
        let mut inner = view.inner.write().await;
        inner.specs.insert(name.to_owned(), live.stored);
        inner
            .servers
            .insert(name.to_owned(), live.peer.borrowed(live.peer.peer.clone()));
        drop(inner);
        Ok(Arc::new(view))
    }

    /// Remove the original sandbox on a policy change without touching a
    /// replacement process that may already occupy this session's scope.
    pub async fn revoke_app_stdio(
        &self,
        session: i64,
        name: &str,
        workspace: &Path,
        peer_id: Uuid,
    ) {
        let _lifecycle = self.lifecycle.read().await;
        let operation = self.server_operation_lock(name).await;
        let _operation = operation.lock().await;
        if let Ok(live) = self.app_stdio_peer(session, name, workspace, peer_id).await {
            let retired = self.scoped_servers.lock().await.remove(&live.key);
            if let Some(manager) = retired {
                self.notify_observation();
                manager.shutdown_local().await;
            }
        }
    }

    // All callers hold the owner lifecycle and per-server operation gates.
    async fn app_stdio_peer(
        &self,
        session: i64,
        name: &str,
        workspace: &Path,
        peer_id: Uuid,
    ) -> McpResult<AppStdioPeer> {
        let unavailable = || McpError::ServerNotConnected(name.to_owned());
        if session <= 0 {
            return Err(unavailable());
        }
        let revision = self
            .inner
            .read()
            .await
            .specs
            .get(name)
            .filter(|stored| matches!(stored.spec, ServerSpec::Stdio { .. }))
            .map(|stored| stored.revision)
            .ok_or_else(unavailable)?;
        let scope = format!("session:{session}");
        let (key, manager) = self
            .scoped_servers
            .lock()
            .await
            .iter()
            .find(|(key, _)| {
                key.revision == revision
                    && key.server == name
                    && key.context.scope == scope
                    && key.context.workspace_root == workspace
            })
            .map(|(key, manager)| (key.clone(), manager.clone()))
            .ok_or_else(unavailable)?;
        let peer = manager.get(name).await?;
        if peer.peer_id != peer_id {
            return Err(unavailable());
        }
        let stored = manager
            .inner
            .read()
            .await
            .specs
            .get(name)
            .cloned()
            .ok_or_else(unavailable)?;
        Ok(AppStdioPeer {
            key,
            manager,
            stored,
            peer,
        })
    }

    pub async fn restart_sandbox_scope(
        self: &Arc<Self>,
        name: &str,
        context: McpSandboxContext,
    ) -> McpResult<Arc<Self>> {
        self.sandbox_scope(name, context, SandboxConnection::Restart)
            .await
            .map(|(manager, _)| manager)
    }

    /// Create an independent, authorized process for an MCP manager's lifetime.
    pub async fn for_managed_sandbox(
        self: &Arc<Self>,
        name: &str,
        context: McpSandboxContext,
    ) -> McpResult<Arc<Self>> {
        self.sandbox_scope(name, context, SandboxConnection::Managed)
            .await
            .map(|(manager, _)| manager)
    }

    /// Activate a channel on the same scoped transport its tools will use.
    pub async fn for_claude_channel_scope(
        self: &Arc<Self>,
        name: &str,
        context: McpSandboxContext,
        authorized_spec: &ServerSpec,
    ) -> McpResult<(Arc<Self>, ClaudeChannelStream)> {
        let (manager, stream) = self
            .sandbox_scope(name, context, SandboxConnection::Channel(authorized_spec))
            .await?;
        Ok((manager, stream.ok_or(McpError::TransportClosed)?))
    }

    async fn sandbox_scope(
        self: &Arc<Self>,
        name: &str,
        context: McpSandboxContext,
        connection: SandboxConnection<'_>,
    ) -> McpResult<(Arc<Self>, Option<ClaudeChannelStream>)> {
        let cache = connection != SandboxConnection::Managed;
        if context.scope.is_empty() || context.scope.len() > 4096 || !context.cwd.is_absolute() {
            return Err(missing_scope(name));
        }
        let _lifecycle = self.lifecycle.read().await;
        let operation = self.server_operation_lock(name).await;
        let _operation = operation.lock().await;
        let stored = self
            .inner
            .read()
            .await
            .specs
            .get(name)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))?;
        if stored.plugin_scoped && (context.policy.disabled || !agena_process::sandbox::enabled()) {
            return Err(McpError::Transport(
                "MCP servers from agent plugins require Agena process sandboxing".into(),
            ));
        }
        let key = ScopedServerKey {
            server: name.to_owned(),
            revision: stored.revision,
            context,
        };
        let ScopedPluginServer {
            mut spec,
            redactions,
        } = stored.in_workspace(&key.context.workspace_root).await?;
        if let SandboxConnection::Channel(authorized_spec) = connection
            && &spec != authorized_spec
        {
            return Err(McpError::PermissionDenied);
        }
        spec.bind_stdio_sandbox(&key.context)?;
        let cached = if cache {
            self.scoped_servers.lock().await.get(&key).cloned()
        } else {
            None
        };
        if let Some(manager) = cached {
            if manager
                .server_spec(name)
                .await
                .is_ok_and(|current| current == spec)
            {
                let stream = manager.connect_sandbox(name, connection).await?;
                return Ok((manager, stream));
            }
            self.scoped_servers.lock().await.remove(&key);
            manager.shutdown_local().await;
        }
        // A settings change cannot keep a more permissive process reachable
        // under the same owning scope. Complete equality decides reuse.
        let retired = if cache {
            let mut instances = self.scoped_servers.lock().await;
            let stale = instances
                .keys()
                .filter(|other| {
                    other.server == name
                        && other.context.scope == key.context.scope
                        && **other != key
                })
                .cloned()
                .collect::<Vec<_>>();
            let retired = stale
                .into_iter()
                .filter_map(|key| instances.remove(&key))
                .collect::<Vec<_>>();
            if instances.len() >= 128 {
                return Err(McpError::Transport("MCP sandbox instance limit reached; close unused scopes before starting another service".into()));
            }
            retired
        } else {
            Vec::new()
        };
        for manager in retired {
            manager.shutdown_local().await;
        }
        let root = if stored.plugin_scoped {
            &key.context.workspace_root
        } else {
            &key.context.cwd
        };
        let mut manager = McpConnectionManager::new(&self.client_name, &self.client_version)
            .with_apps(self.apps_enabled)
            .with_roots(
                root.is_absolute()
                    .then(|| root.clone())
                    .into_iter()
                    .chain(key.context.additional_working_directories.iter().cloned()),
            )
            .with_timeouts(stored.connect_timeout, stored.request_timeout);
        manager.observations = self.observations.clone();
        let manager = Arc::new(manager);
        manager.inner.write().await.specs.insert(
            name.to_owned(),
            StoredServerSpec {
                spec,
                resolve_workspace: None,
                redactions,
                ..stored
            },
        );
        let stream = match manager.connect_sandbox(name, connection).await {
            Ok(stream) => stream,
            Err(error) => {
                manager.shutdown_local().await;
                return Err(error);
            }
        };
        // Reconnection requires another authenticated call, which refreshes
        // persisted denials and policy ceilings before reaching this cache.
        if cache {
            self.scoped_servers
                .lock()
                .await
                .insert(key, Arc::clone(&manager));
        }
        self.notify_observation();
        Ok((manager, stream))
    }

    async fn connect_sandbox(
        &self,
        name: &str,
        connection: SandboxConnection<'_>,
    ) -> McpResult<Option<ClaudeChannelStream>> {
        if matches!(connection, SandboxConnection::Channel(_)) {
            return self.open_claude_channel(name).await.map(Some);
        }
        if connection == SandboxConnection::Restart || self.get(name).await.is_err() {
            self.reconnect(name).await?;
        }
        Ok(None)
    }

    /// Retire the exact server manager the caller owns without closing other
    /// channels or an instance that replaced it after a policy change.
    pub async fn release_sandbox_server(
        &self,
        server: &str,
        connection: &Arc<McpConnectionManager>,
    ) {
        let _lifecycle = self.lifecycle.read().await;
        let operation = self.server_operation_lock(server).await;
        let _operation = operation.lock().await;
        let retired = self
            .scoped_servers
            .lock()
            .await
            .extract_if(|key, manager| key.server == server && Arc::ptr_eq(manager, connection))
            .map(|(_, manager)| manager)
            .collect::<Vec<_>>();
        for manager in retired {
            self.notify_observation();
            manager.shutdown_local().await;
        }
    }

    /// One-call approvals are retired when their host-issued completion fires.
    pub async fn release_sandbox_scope(&self, scope: &str) {
        let _lifecycle = self.lifecycle.read().await;
        let retired = {
            let mut instances = self.scoped_servers.lock().await;
            let keys = instances
                .keys()
                .filter(|key| key.context.scope == scope)
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| instances.remove(&key))
                .collect::<Vec<_>>()
        };
        if !retired.is_empty() {
            self.notify_observation();
        }
        for manager in retired {
            manager.shutdown_local().await;
        }
    }

    /// Retire the connections and tool selections owned by this conversation.
    /// The lifecycle gate puts completed launches on the same side of the drain
    /// whether they were started by a command or by an ordinary MCP tool call.
    pub async fn release_session(&self, session: i64) {
        if session <= 0 {
            return;
        }
        let _lifecycle = self.lifecycle.write().await;
        self.retire_app_http_connections(|owner, _| owner == session)
            .await;
        self.retire_managed_connections(|owner, _| owner == session)
            .await;
        let scope = format!("session:{session}");
        let call_prefix = format!("{scope}:call:");
        let stdio = self
            .scoped_servers
            .lock()
            .await
            .extract_if(|key, _| {
                key.context.scope == scope || key.context.scope.starts_with(&call_prefix)
            })
            .map(|(_, manager)| manager)
            .collect::<Vec<_>>();
        let http = self
            .plugin_http_calls
            .lock()
            .await
            .extract_if(.., |key, _| key.session == session)
            .map(|(_, manager)| manager)
            .collect::<Vec<_>>();
        self.clear_on_demand_session(session).await;
        if !stdio.is_empty() || !http.is_empty() {
            self.notify_observation();
        }
        for manager in stdio.into_iter().chain(http) {
            manager.shutdown_local().await;
        }
    }

    pub(super) async fn retire_scoped_server(&self, server: &str) {
        let retired = {
            let mut instances = self.scoped_servers.lock().await;
            let keys = instances
                .keys()
                .filter(|key| key.server == server)
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| instances.remove(&key))
                .collect::<Vec<_>>()
        };
        let calls = {
            let mut calls = self.plugin_http_calls.lock().await;
            calls
                .extract_if(.., |key, _| key.server == server)
                .map(|(_, manager)| manager)
                .collect::<Vec<_>>()
        };
        for manager in retired.into_iter().chain(calls) {
            manager.shutdown_local().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deferred() -> ServerSpec {
        ServerSpec::Stdio {
            command: "a-program-that-must-not-be-launched".into(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            tool_policy: Default::default(),
            resources: Default::default(),
            sandbox: None,
        }
    }

    async fn configured() -> Arc<McpConnectionManager> {
        let manager = Arc::new(McpConnectionManager::default());
        manager.add_server("fixture", deferred()).await.unwrap();
        manager
    }

    fn key(scope: &str) -> ScopedServerKey {
        ScopedServerKey {
            server: "fixture".into(),
            revision: 1,
            context: McpSandboxContext {
                scope: scope.into(),
                workspace_root: PathBuf::from("/project"),
                cwd: PathBuf::from("/project"),
                additional_working_directories: Vec::new(),
                policy: SandboxPolicy {
                    readonly_paths: vec![PathBuf::from("/project")],
                    ..Default::default()
                },
            },
        }
    }

    #[test]
    fn cache_key_keeps_scope_revision_and_every_policy_field() {
        let original = key("session:1");
        assert_ne!(original, key("session:2"));
        let mut other = original.clone();
        other.revision += 1;
        assert_ne!(original, other);
        let mut other = original.clone();
        other
            .context
            .policy
            .denied_paths
            .push("/project/secret".into());
        assert_ne!(original, other);
        let mut other = original.clone();
        other
            .context
            .policy
            .readwrite_paths
            .push("/project/out".into());
        assert_ne!(original, other);
        let mut other = original.clone();
        other.context.workspace_root = "/other".into();
        assert_ne!(original, other);
        let mut other = original.clone();
        other.context.policy.timeout_ms = Some(1);
        assert_ne!(original, other);
    }

    #[tokio::test]
    async fn configured_stdio_is_deferred_and_cannot_run_without_scope() {
        let manager = McpConnectionManager::default();
        manager
            .add_server(
                "fixture",
                ServerSpec::Stdio {
                    command: "a-program-that-must-not-be-launched".into(),
                    args: vec![],
                    env: HashMap::new(),
                    cwd: None,
                    tool_policy: Default::default(),
                    resources: Default::default(),
                    sandbox: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(manager.server_names().await, ["fixture"]);
        assert!(!manager.statuses().await[0].connected);
        assert!(manager.disconnected_server_names().await.is_empty());
        assert!(
            manager
                .reconnect("fixture")
                .await
                .unwrap_err()
                .to_string()
                .contains("execution scope")
        );
        assert!(
            manager
                .call_tool("fixture", "read", None)
                .await
                .unwrap_err()
                .to_string()
                .contains("execution scope")
        );
        manager.shutdown_all().await;
    }

    #[tokio::test]
    async fn retiring_one_call_scope_preserves_other_sessions() {
        let owner = configured().await;
        let first = configured().await;
        let second = configured().await;
        owner
            .scoped_servers
            .lock()
            .await
            .insert(key("session:1:call:3"), Arc::clone(&first));
        owner
            .scoped_servers
            .lock()
            .await
            .insert(key("session:2"), Arc::clone(&second));
        owner.release_sandbox_scope("session:1:call:3").await;
        assert!(first.server_names().await.is_empty());
        assert_eq!(second.server_names().await, ["fixture"]);
        assert_eq!(owner.scoped_servers.lock().await.len(), 1);
        owner.shutdown_all().await;
        assert!(second.server_names().await.is_empty());
        assert!(owner.scoped_servers.lock().await.is_empty());
    }

    #[tokio::test]
    async fn retiring_a_session_drains_its_stdio_and_http_scopes() {
        let owner = configured().await;
        let first = configured().await;
        let call = configured().await;
        let http = configured().await;
        let sibling = configured().await;
        owner.scoped_servers.lock().await.extend([
            (key("session:1"), Arc::clone(&first)),
            (key("session:1:call:3:origin:one"), Arc::clone(&call)),
            (key("session:11"), Arc::clone(&sibling)),
        ]);
        owner.plugin_http_calls.lock().await.extend([
            (
                PluginHttpCallKey {
                    server: "fixture".into(),
                    session: 1,
                    origin: "one".into(),
                },
                Arc::clone(&http),
            ),
            (
                PluginHttpCallKey {
                    server: "fixture".into(),
                    session: 11,
                    origin: "eleven".into(),
                },
                Arc::clone(&sibling),
            ),
        ]);
        owner.release_session(1).await;
        for scope in [first, call, http] {
            assert!(scope.server_names().await.is_empty());
        }
        assert_eq!(sibling.server_names().await, ["fixture"]);
        assert_eq!(owner.scoped_servers.lock().await.len(), 1);
        assert_eq!(owner.plugin_http_calls.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn replacing_configuration_retires_all_old_policy_instances() {
        let owner = configured().await;
        let child = configured().await;
        let revision = owner.inner.read().await.specs["fixture"].revision;
        let mut old_key = key("session:1");
        old_key.revision = revision;
        owner
            .scoped_servers
            .lock()
            .await
            .insert(old_key, Arc::clone(&child));
        owner.add_server("fixture", deferred()).await.unwrap();
        assert!(owner.inner.read().await.specs["fixture"].revision > revision);
        assert!(child.server_names().await.is_empty());
        assert!(owner.scoped_servers.lock().await.is_empty());
        owner.shutdown_all().await;
    }

    #[tokio::test]
    async fn claude_channel_rejects_a_replaced_server_before_launch() {
        let owner = configured().await;
        let authorized = owner.server_spec("fixture").await.unwrap();
        let mut replacement = authorized.clone();
        if let ServerSpec::Stdio { args, .. } = &mut replacement {
            args.push("new-launch-argument".into());
        }
        owner.add_server("fixture", replacement).await.unwrap();
        let mut context = key("session:1").context;
        context.cwd = std::env::current_dir().unwrap();
        context.workspace_root = context.cwd.clone();
        assert!(matches!(
            owner
                .for_claude_channel_scope("fixture", context, &authorized)
                .await,
            Err(McpError::PermissionDenied),
        ));
        assert!(owner.scoped_servers.lock().await.is_empty());
    }

    #[tokio::test]
    async fn unscoped_connection_probe_is_an_explicit_failure() {
        let result = crate::management::test_connection(
            "fixture",
            deferred(),
            PathBuf::from("/project"),
            50,
            50,
            false,
        )
        .await;
        assert!(!result.ok);
        assert!(result.error.unwrap().contains("sandbox scope"));
    }
    struct HttpAuthority;

    #[async_trait::async_trait]
    impl crate::McpHttpNetworkAuthorizer for HttpAuthority {
        async fn authorize(&self, _target: &str) -> McpResult<()> {
            Ok(())
        }
    }

    struct FixtureTokens;
    impl TokenStore for FixtureTokens {
        fn bearer(&self, _server: &str) -> Option<String> {
            Some("fixture-token".into())
        }
    }

    #[tokio::test]
    async fn agent_plugin_http_cannot_connect_before_scope_and_uses_the_caller_workspace() {
        let manager = McpConnectionManager::default();
        let spec = |workspace: &Path| -> Result<ScopedPluginServer, String> {
            Ok(ServerSpec::Http {
                header_helper: None,
                url: Url::parse("http://127.0.0.1:1/mcp").unwrap(),
                transport: Default::default(),
                headers: HashMap::from([("x-workspace".into(), workspace.display().to_string())]),
                auth: None,
                tool_policy: Default::default(),
            }
            .into())
        };
        let initial = spec(Path::new("default-workspace")).unwrap();
        manager
            .register_scoped_plugin_server(
                "plugin-server",
                initial.clone(),
                super::ScopedPluginTimeouts::default(),
                Some(Arc::new(spec)),
            )
            .await
            .unwrap();
        assert!(manager.reconnect("plugin-server").await.is_err());
        assert!(manager.disconnected_server_names().await.is_empty());
        assert!(manager.statuses().await[0].scope_required);
        assert!(
            manager
                .register_scoped_plugin_server(
                    "plugin-server",
                    initial,
                    super::ScopedPluginTimeouts::default(),
                    None,
                )
                .await
                .is_err()
        );

        let scoped = manager
            .for_http_scope_in_workspace(
                "plugin-server",
                Arc::new(HttpAuthority),
                Some(Path::new("selected-workspace")),
            )
            .await
            .unwrap();
        let ServerSpec::Http { headers, .. } = scoped.server_spec("plugin-server").await.unwrap()
        else {
            panic!("expected HTTP spec");
        };
        assert_eq!(headers["x-workspace"], "selected-workspace");
        assert!(!scoped.statuses().await[0].scope_required);
    }

    #[tokio::test]
    async fn http_scopes_are_disconnected_isolated_and_keep_auth_configuration() {
        let mut owner = McpConnectionManager::default();
        owner.set_token_store(Arc::new(FixtureTokens));
        owner.inner.write().await.specs.insert(
            "http".into(),
            StoredServerSpec {
                spec: ServerSpec::Http {
                    header_helper: None,
                    url: Url::parse("http://scope-fixture.invalid/mcp").unwrap(),
                    transport: Default::default(),
                    headers: HashMap::new(),
                    auth: Some(HttpAuth::BearerFromStore),
                    tool_policy: Default::default(),
                },
                connect_timeout: Duration::from_millis(17),
                request_timeout: Duration::from_millis(19),
                plugin_timeouts: super::ScopedPluginTimeouts::default(),
                revision: 7,
                plugin_scoped: false,
                resolve_workspace: None,
                redactions: McpRedactions::default(),
                on_demand: None,
            },
        );
        let authority: Arc<dyn crate::McpHttpNetworkAuthorizer> = Arc::new(HttpAuthority);
        let first = owner
            .for_http_scope("http", authority.clone())
            .await
            .unwrap();
        let second = owner.for_http_scope("http", authority).await.unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&owner.inner, &first.inner));
        assert!(first.inner.read().await.servers.is_empty());
        assert!(first.http_network_authorizer.is_some());
        assert!(owner.http_network_authorizer.is_none());
        assert!(Arc::ptr_eq(
            first.token_store.as_ref().unwrap(),
            owner.token_store.as_ref().unwrap()
        ));
        assert_eq!(first.connect_timeout, Duration::from_millis(17));
        assert_eq!(first.request_timeout, Duration::from_millis(19));
        first.shutdown_all().await;
        assert!(second.server_spec("http").await.is_ok());
        assert!(owner.server_spec("http").await.is_ok());
    }
}
