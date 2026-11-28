use super::*;
use crate::http_transport::ScopedHttpRequest;

impl McpConnectionManager {
    /// Borrow one connected MCP peer for a single invocation. A managed
    /// background connection requires its own authority as well. App peers
    /// retain the connection while executing requests under the new caller.
    pub async fn borrow_http(
        &self,
        name: &str,
        authority: Arc<dyn crate::McpHttpNetworkAuthorizer>,
        require_connection_authority: bool,
    ) -> McpResult<Arc<Self>> {
        if self.http_network_authorizer.is_none()
            || (!require_connection_authority && self.http_background_enabled)
        {
            return Err(McpError::PermissionDenied);
        }
        let (stored, server) = {
            let _lifecycle = self.lifecycle.read().await;
            let inner = self.inner.read().await;
            let stored = inner
                .specs
                .get(name)
                .filter(|stored| match &stored.spec {
                    ServerSpec::Http { transport, .. } => {
                        !require_connection_authority || *transport == McpHttpTransport::Streamable
                    }
                    ServerSpec::Stdio { .. } => false,
                })
                .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))?;
            let server = inner
                .servers
                .get(name)
                .filter(|server| {
                    !server.peer.is_transport_closed() && !server.events.shutdown.is_cancelled()
                })
                .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))?;
            (stored.clone(), server.clone())
        };
        // One operation at a time lets callback replies inherit the authority
        // of their unique in-flight request on a legacy receive stream.
        let legacy_operation = if matches!(
            &stored.spec,
            ServerSpec::Http {
                transport: McpHttpTransport::Sse | McpHttpTransport::WebSocket,
                ..
            }
        ) {
            let operation = self.server_operation_lock(name).await;
            let cancelled = authority.cancellation().unwrap_or_default();
            Some(tokio::select! {
                biased;
                _ = cancelled.cancelled() => return Err(McpError::PermissionDenied),
                guard = operation.lock_owned() => guard,
            })
        } else {
            None
        };
        if server.peer.is_transport_closed() || server.events.shutdown.is_cancelled() {
            return Err(McpError::ServerNotConnected(name.to_owned()));
        }
        let peer = server
            .peer
            .clone()
            .with_outbound_extension(ScopedHttpRequest {
                authority: authority.clone(),
                require_connection_authority,
            });
        let borrowed = server.borrowed(peer);
        let mut manager = Self::new(&self.client_name, &self.client_version)
            .with_apps(self.apps_enabled)
            .with_http_network_authorizer(authority);
        manager.roots = self.roots.clone();
        manager.borrowed_http = true;
        manager._legacy_app_operation = legacy_operation;
        let mut inner = manager.inner.write().await;
        inner.specs.insert(name.to_owned(), stored);
        inner.servers.insert(name.to_owned(), borrowed);
        drop(inner);
        Ok(Arc::new(manager))
    }
}
