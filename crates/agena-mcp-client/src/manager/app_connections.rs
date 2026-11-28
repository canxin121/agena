use futures::StreamExt;
use rmcp::model::ServerJsonRpcMessage;
use tokio::time::Instant;

use super::*;
use crate::McpHttpNetworkAuthorizer;

const APP_CONNECTION_LIMIT: usize = 64;
const APP_IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub(super) struct AppHttpConnection {
    server: String,
    workspace: PathBuf,
    last_used: Instant,
    manager: Arc<McpConnectionManager>,
}

impl McpConnectionManager {
    /// Observe catalog invalidations on the already authorized peer without
    /// retaining its connection or issuing a request.
    pub async fn observe_tool_list_changes(
        &self,
        name: &str,
    ) -> McpResult<Option<(watch::Receiver<u64>, CancellationToken)>> {
        let server = self.get(name).await?;
        let supported = server.peer.peer_info().is_some_and(|info| {
            info.capabilities
                .tools
                .as_ref()
                .is_some_and(|tools| tools.list_changed == Some(true))
        });
        Ok(supported.then(|| {
            (
                server.events.tool_list_changes.subscribe(),
                server.events.shutdown.clone(),
            )
        }))
    }

    /// Observe the resource catalog on the original peer, even after the
    /// authorized request has released its transport borrow.
    pub async fn observe_resource_list_changes(
        &self,
        name: &str,
    ) -> McpResult<Option<(Uuid, watch::Receiver<u64>, CancellationToken)>> {
        let server = self.get(name).await?;
        let supported = server.peer.peer_info().is_some_and(|info| {
            info.capabilities
                .resources
                .as_ref()
                .is_some_and(|resources| resources.list_changed == Some(true))
        });
        Ok(supported.then(|| {
            (
                server.peer_id,
                server.events.resource_list_changes.subscribe(),
                server.events.shutdown.clone(),
            )
        }))
    }

    pub async fn app_resource_catalog_generation(
        &self,
        name: &str,
    ) -> McpResult<Option<(u64, bool)>> {
        let server = self.get(name).await?;
        Ok(server.peer.peer_info().and_then(|info| {
            info.capabilities.resources.as_ref().map(|resources| {
                (
                    *server.events.resource_list_changes.borrow(),
                    resources.list_changed == Some(true),
                )
            })
        }))
    }

    /// Wait on an App's original Streamable HTTP peer with the live guest
    /// request's authority. Only one receive stream is needed per peer.
    pub async fn wait_for_app_http_resource_change(
        &self,
        name: &str,
        generation: u64,
        timeout: Duration,
        authority: &dyn McpHttpNetworkAuthorizer,
    ) -> McpResult<bool> {
        if !self.borrowed_http {
            return Err(McpError::PermissionDenied);
        }
        let server = self.get(name).await?;
        let Some(info) = server.peer.peer_info().filter(|info| {
            info.capabilities
                .resources
                .as_ref()
                .is_some_and(|resources| resources.list_changed == Some(true))
        }) else {
            return Ok(false);
        };
        let modern = info.protocol_version >= ProtocolVersion::V_2026_07_28;
        drop(info);
        let events = &server.events;
        let mut changes = events.resource_list_changes.subscribe();
        if *changes.borrow_and_update() != generation {
            return Ok(true);
        }
        let observe = async {
            let Ok(_listen) = events.app_resource_listener.try_lock() else {
                return changes
                    .changed()
                    .await
                    .map(|()| true)
                    .map_err(|_| McpError::TransportClosed);
            };
            let next = if modern {
                next_subscription_resource_change(&server.peer).await
            } else {
                next_legacy_resource_change(events, authority).await
            };
            match next {
                Ok(true) => {
                    events.resource_list_changed();
                    Ok(true)
                }
                Ok(false) => Ok(false),
                Err(McpError::PermissionDenied) => Err(McpError::PermissionDenied),
                Err(error) => {
                    tracing::debug!(server = name, error = %error.user_message(), "MCP App catalog stream unavailable");
                    Ok(false)
                }
            }
        };
        tokio::select! {
            biased;
            _ = events.shutdown.cancelled() => Err(McpError::TransportClosed),
            result = tokio::time::timeout(timeout, observe) => result.unwrap_or(Ok(true)),
        }
    }

    /// Keep the exact network peer that produced this App's result.
    /// Its request-scoped transport can be used only with fresh authority.
    pub async fn retain_app_http_connection(
        self: &Arc<Self>,
        session: i64,
        call: i64,
        server: &str,
        workspace: &Path,
        manager: Arc<Self>,
    ) -> Option<Arc<Self>> {
        let _lifecycle = self.lifecycle.read().await;
        if manager.http_background_enabled || manager.borrowed_http || Arc::ptr_eq(&manager, self) {
            return None;
        }
        let operation = self.server_operation_lock(server).await;
        let _operation = operation.lock().await;
        let revision = manager
            .inner
            .read()
            .await
            .specs
            .get(server)
            .map(|spec| spec.revision);
        let current = self
            .inner
            .read()
            .await
            .specs
            .get(server)
            .map(|spec| spec.revision);
        if revision.is_none() || revision != current {
            return None;
        }
        let now = Instant::now();
        let retired = {
            let mut entries = self.app_http_connections.lock().await;
            if let Some(existing) = entries.get_mut(&(session, call))
                && existing.server == server
                && existing.workspace == workspace
            {
                existing.last_used = now;
                return Some(existing.manager.clone());
            }
            let mut retired = Vec::new();
            if entries.len() >= APP_CONNECTION_LIMIT {
                let oldest = entries
                    .iter()
                    .filter(|(_, entry)| Arc::strong_count(&entry.manager) == 1)
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(&key, _)| key);
                let Some(oldest) = oldest else {
                    return None;
                };
                retired.extend(entries.remove(&oldest));
            }
            retired.extend(entries.insert(
                (session, call),
                AppHttpConnection {
                    server: server.to_owned(),
                    workspace: workspace.to_owned(),
                    last_used: now,
                    manager: manager.clone(),
                },
            ));
            retired
        };
        for entry in retired {
            entry.manager.shutdown_local().await;
        }

        let owner = Arc::downgrade(self);
        let retained = Arc::downgrade(&manager);
        tokio::spawn(async move {
            let mut deadline = now + APP_IDLE_TIMEOUT;
            loop {
                tokio::time::sleep_until(deadline).await;
                let Some(owner) = owner.upgrade() else { break };
                let expired = {
                    let mut entries = owner.app_http_connections.lock().await;
                    let Some(entry) = entries.get_mut(&(session, call)) else {
                        break;
                    };
                    if !retained.ptr_eq(&Arc::downgrade(&entry.manager)) {
                        break;
                    }
                    deadline = entry.last_used + APP_IDLE_TIMEOUT;
                    if deadline > Instant::now() {
                        continue;
                    }
                    if Arc::strong_count(&entry.manager) > 1 {
                        entry.last_used = Instant::now();
                        deadline = entry.last_used + APP_IDLE_TIMEOUT;
                        continue;
                    }
                    entries.remove(&(session, call))
                };
                if let Some(entry) = expired {
                    entry.manager.shutdown_local().await;
                }
                break;
            }
        });
        Some(manager)
    }

    /// Stdio and legacy streams are connection-bound. Streamable HTTP without
    /// a session ID can reopen a stateless App after the connection is released.
    pub async fn app_requires_original_peer(&self, server: &str) -> bool {
        let inner = self.inner.read().await;
        let Some(stored) = inner.specs.get(server) else {
            return false;
        };
        match &stored.spec {
            ServerSpec::Http {
                transport: McpHttpTransport::Sse | McpHttpTransport::WebSocket,
                ..
            } => true,
            ServerSpec::Http {
                transport: McpHttpTransport::Streamable,
                ..
            } => inner.servers.get(server).is_some_and(|connected| {
                connected.events.http_session_open.load(Ordering::Acquire)
            }),
            ServerSpec::Stdio { .. } => true,
        }
    }

    pub async fn app_http_connection(
        &self,
        session: i64,
        call: i64,
        server: &str,
        workspace: &Path,
    ) -> Option<Arc<Self>> {
        let _lifecycle = self.lifecycle.read().await;
        let mut entries = self.app_http_connections.lock().await;
        let entry = entries
            .get_mut(&(session, call))
            .filter(|entry| entry.server == server && entry.workspace == workspace)?;
        entry.last_used = Instant::now();
        Some(entry.manager.clone())
    }

    /// Revoke only the peer that was checked. A concurrent retirement or
    /// replacement of the entry cannot close an unrelated connection.
    pub async fn revoke_app_http_connection(
        &self,
        session: i64,
        call: i64,
        connection: &Arc<Self>,
    ) {
        let _lifecycle = self.lifecycle.read().await;
        let retired = {
            let mut entries = self.app_http_connections.lock().await;
            let key = (session, call);
            if entries
                .get(&key)
                .is_some_and(|entry| Arc::ptr_eq(&entry.manager, connection))
            {
                entries.remove(&key)
            } else {
                None
            }
        };
        if let Some(entry) = retired {
            entry.manager.shutdown_local().await;
        }
    }

    pub(super) async fn retire_app_http_connections(&self, remove: impl Fn(i64, &str) -> bool) {
        let retired = self
            .app_http_connections
            .lock()
            .await
            .extract_if(.., |(session, _), entry| remove(*session, &entry.server))
            .map(|(_, entry)| entry.manager)
            .collect::<Vec<_>>();
        for manager in retired {
            manager.shutdown_local().await;
        }
    }
}

async fn next_subscription_resource_change(peer: &Peer<RoleClient>) -> McpResult<bool> {
    let mut filter = SubscriptionFilter::default();
    filter.resources_list_changed = Some(true);
    let mut subscription = peer.listen(filter).await?;
    if subscription.acknowledged().resources_list_changed != Some(true) {
        return Ok(false);
    }
    while let Some(event) = subscription.next().await? {
        if matches!(
            event,
            ServerNotification::ResourceListChangedNotification(_)
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn next_legacy_resource_change(
    events: &ServerEventState,
    authority: &dyn McpHttpNetworkAuthorizer,
) -> McpResult<bool> {
    let Some(mut stream) = events.app_http_events.open_legacy(authority).await? else {
        return Ok(false);
    };
    while let Some(event) = stream.next().await {
        let event = event
            .map_err(|_| McpError::Transport("MCP App resource event stream closed".into()))?;
        if event.data.as_deref().is_some_and(|data| matches!(
            serde_json::from_str::<ServerJsonRpcMessage>(data),
            Ok(ServerJsonRpcMessage::Notification(notification))
                if matches!(notification.notification, ServerNotification::ResourceListChangedNotification(_))
        )) {
            return Ok(true);
        }
    }
    Ok(false)
}
