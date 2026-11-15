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
    pub policy: SandboxPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ScopedServerKey {
    server: String,
    revision: u64,
    context: McpSandboxContext,
}

pub(super) fn missing_scope(server: &str) -> McpError {
    McpError::Transport(format!(
        "MCP stdio server `{server}` requires a host-authorized execution scope"
    ))
}

impl McpConnectionManager {
    pub async fn server_spec(&self, name: &str) -> McpResult<ServerSpec> {
        self.inner
            .read()
            .await
            .specs
            .get(name)
            .map(|stored| stored.spec.clone())
            .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))
    }

    /// Select a process by configuration revision, owning scope, working
    /// directory and the entire kernel policy. A narrower caller never finds
    /// a shared server-name-only process.
    pub async fn for_sandbox_scope(
        self: &Arc<Self>,
        name: &str,
        context: McpSandboxContext,
    ) -> McpResult<Arc<Self>> {
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
        let key = ScopedServerKey {
            server: name.to_owned(),
            revision: stored.revision,
            context,
        };
        let mut spec = stored.spec;
        match &mut spec {
            ServerSpec::Http { .. } => return Ok(Arc::clone(self)),
            ServerSpec::Stdio { cwd, sandbox, .. } => {
                if sandbox.is_some() {
                    return Err(McpError::Transport(
                        "an MCP process cannot expand or rebind its existing sandbox scope".into(),
                    ));
                }
                *cwd = Some(key.context.cwd.clone());
                *sandbox = Some(key.context.policy.clone());
            }
        }
        if let Some(manager) = self.scoped_servers.lock().await.get(&key).cloned() {
            if manager.get(name).await.is_err() {
                manager.reconnect(name).await?;
            }
            return Ok(manager);
        }
        // A settings change cannot keep a more permissive process reachable
        // under the same owning scope. Complete equality decides reuse.
        let retired = {
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
        };
        for manager in retired {
            manager.shutdown_local().await;
        }
        let manager = Arc::new(
            McpConnectionManager::new(&self.client_name, &self.client_version)
                .with_roots([key.context.cwd.clone()])
                .with_timeouts(stored.connect_timeout, stored.request_timeout),
        );
        manager.add_server(name, spec).await?;
        // Reconnection requires another authenticated call, which refreshes
        // persisted denials and policy ceilings before reaching this cache.
        self.scoped_servers
            .lock()
            .await
            .insert(key, Arc::clone(&manager));
        Ok(manager)
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
        for manager in retired {
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
        for manager in retired {
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
}
