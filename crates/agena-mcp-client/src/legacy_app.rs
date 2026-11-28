//! Request authority for legacy transports that retain an App's original peer.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use agena_process::sandbox_http::{HttpOptions, WorkerCancellation};
use rmcp::{
    model::{
        ClientJsonRpcMessage, ClientNotification, GetExtensions, RequestId, ServerJsonRpcMessage,
        ServerNotification,
    },
    service::InboundStreamOrigin,
};

use crate::{McpError, McpHttpNetworkAuthorizer, McpResult, http_transport::ScopedHttpRequest};

const MAX_PENDING: usize = 64;

#[derive(Clone)]
struct RequestAuthority {
    authorizer: Arc<dyn McpHttpNetworkAuthorizer>,
    cancellation: Option<WorkerCancellation>,
}

impl RequestAuthority {
    fn new(authorizer: Arc<dyn McpHttpNetworkAuthorizer>) -> Self {
        Self {
            cancellation: authorizer.cancellation(),
            authorizer,
        }
    }

    fn active(&self) -> bool {
        !self
            .cancellation
            .as_ref()
            .is_some_and(WorkerCancellation::is_cancelled)
    }
}

#[derive(Default)]
struct PendingMessages {
    requests: HashMap<RequestId, RequestAuthority>,
    callbacks: HashMap<RequestId, RequestId>,
}

impl PendingMessages {
    fn prune(&mut self) {
        self.requests.retain(|_, authority| authority.active());
        self.callbacks
            .retain(|_, origin| self.requests.contains_key(origin));
    }

    fn reply_authority(&mut self, id: &RequestId) -> Option<RequestAuthority> {
        let request = self.callbacks.remove(id)?;
        self.requests.get(&request).cloned()
    }
}

pub(crate) struct LegacyAppPeer {
    address: SocketAddr,
    sandbox_disabled: bool,
    lifetime: WorkerCancellation,
    pending: Mutex<PendingMessages>,
}

impl LegacyAppPeer {
    pub(crate) fn new(
        address: SocketAddr,
        sandbox_disabled: bool,
        lifetime: WorkerCancellation,
    ) -> Self {
        Self {
            address,
            sandbox_disabled,
            lifetime,
            pending: Mutex::default(),
        }
    }

    pub(crate) async fn authorize_send(
        &self,
        target: &str,
        setup: &Arc<dyn McpHttpNetworkAuthorizer>,
        message: &ClientJsonRpcMessage,
    ) -> McpResult<HttpOptions> {
        let authority = self.outbound_authority(setup, message)?;
        if self.lifetime.is_cancelled() || !authority.active() {
            return Err(McpError::PermissionDenied);
        }
        authority.authorizer.authorize(target).await?;
        if !authority
            .authorizer
            .authorize_resolved(target, self.address)
            .await?
        {
            self.lifetime.cancel();
            return Err(McpError::PermissionDenied);
        }
        let policy = authority.authorizer.sandbox_policy(target).await?;
        if policy.disabled != self.sandbox_disabled {
            self.lifetime.cancel();
            return Err(McpError::PermissionDenied);
        }
        if !authority.active() {
            return Err(McpError::PermissionDenied);
        }
        if let ClientJsonRpcMessage::Request(request) = message {
            let mut pending = self.pending.lock().map_err(|_| McpError::TransportClosed)?;
            pending.prune();
            if pending.requests.len() >= MAX_PENDING {
                return Err(McpError::Transport(
                    "too many MCP App requests on this peer".into(),
                ));
            }
            pending
                .requests
                .insert(request.id.clone(), authority.clone());
        }
        Ok(HttpOptions {
            policy,
            pinned_addresses: vec![self.address],
            connect_timeout: Duration::from_secs(20),
            cancellation: authority.cancellation,
            ..HttpOptions::default()
        })
    }

    fn outbound_authority(
        &self,
        setup: &Arc<dyn McpHttpNetworkAuthorizer>,
        message: &ClientJsonRpcMessage,
    ) -> McpResult<RequestAuthority> {
        let mut pending = self.pending.lock().map_err(|_| McpError::TransportClosed)?;
        pending.prune();
        match message {
            ClientJsonRpcMessage::Response(response) => pending
                .reply_authority(&response.id)
                .ok_or(McpError::PermissionDenied),
            ClientJsonRpcMessage::Error(error) => error
                .id
                .as_ref()
                .and_then(|id| pending.reply_authority(id))
                .ok_or(McpError::PermissionDenied),
            _ => match ScopedHttpRequest::from_message(message) {
                Some(scope) if scope.require_connection_authority => {
                    Err(McpError::PermissionDenied)
                }
                Some(scope) => Ok(RequestAuthority::new(scope.authority.clone())),
                None => Ok(RequestAuthority::new(setup.clone())),
            },
        }
    }

    pub(crate) fn sent(&self, message: &ClientJsonRpcMessage, succeeded: bool) {
        let id = match message {
            ClientJsonRpcMessage::Request(request) if !succeeded => Some(&request.id),
            ClientJsonRpcMessage::Notification(notification) => match &notification.notification {
                ClientNotification::CancelledNotification(cancelled) => {
                    cancelled.params.request_id.as_ref()
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(id) = id
            && let Ok(mut pending) = self.pending.lock()
        {
            pending.requests.remove(id);
            pending.prune();
        }
    }

    pub(crate) fn receive(&self, message: &mut ServerJsonRpcMessage) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        pending.prune();
        match message {
            ServerJsonRpcMessage::Request(request) => {
                // These transports carry no response-stream identifier. A
                // callback is safe to associate only with one live request.
                if pending.requests.len() != 1
                    || pending.callbacks.len() >= MAX_PENDING
                    || pending.callbacks.contains_key(&request.id)
                {
                    return false;
                }
                let origin = pending
                    .requests
                    .keys()
                    .next()
                    .expect("one active request")
                    .clone();
                request
                    .request
                    .extensions_mut()
                    .insert(InboundStreamOrigin::OutboundRequest(origin.clone()));
                pending.callbacks.insert(request.id.clone(), origin);
            }
            ServerJsonRpcMessage::Response(response) => {
                pending.requests.remove(&response.id);
            }
            ServerJsonRpcMessage::Error(error) => {
                if let Some(id) = &error.id {
                    pending.requests.remove(id);
                }
            }
            ServerJsonRpcMessage::Notification(notification) => {
                if let ServerNotification::CancelledNotification(cancelled) =
                    &notification.notification
                    && let Some(id) = &cancelled.params.request_id
                {
                    pending.requests.remove(id);
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_process::sandbox::SandboxPolicy;
    use serde_json::json;

    const TARGET: &str = "https://example.test/mcp";

    struct Authority {
        address: Option<SocketAddr>,
        sandbox_disabled: bool,
        cancellation: WorkerCancellation,
    }

    #[async_trait::async_trait]
    impl McpHttpNetworkAuthorizer for Authority {
        fn cancellation(&self) -> Option<WorkerCancellation> {
            Some(self.cancellation.child_token())
        }

        async fn authorize(&self, target: &str) -> McpResult<()> {
            assert_eq!(target, TARGET);
            self.address.map(|_| ()).ok_or(McpError::PermissionDenied)
        }

        async fn authorize_resolved(&self, target: &str, address: SocketAddr) -> McpResult<bool> {
            assert_eq!(target, TARGET);
            Ok(self.address == Some(address))
        }

        async fn sandbox_policy(&self, target: &str) -> McpResult<SandboxPolicy> {
            assert_eq!(target, TARGET);
            Ok(SandboxPolicy {
                disabled: self.sandbox_disabled,
                ..SandboxPolicy::default()
            })
        }
    }

    fn authority(address: Option<SocketAddr>, sandbox_disabled: bool) -> Arc<Authority> {
        Arc::new(Authority {
            address,
            sandbox_disabled,
            cancellation: WorkerCancellation::new(),
        })
    }

    fn message(id: u32, authority: Arc<Authority>) -> ClientJsonRpcMessage {
        let mut message: ClientJsonRpcMessage = serde_json::from_value(json!({
            "jsonrpc":"2.0","id":id,"method":"ping"
        }))
        .unwrap();
        let ClientJsonRpcMessage::Request(request) = &mut message else {
            panic!("ping request")
        };
        request.request.extensions_mut().insert(ScopedHttpRequest {
            authority,
            require_connection_authority: false,
        });
        message
    }

    #[tokio::test]
    async fn legacy_app_peer_checks_the_connected_address_and_current_sandbox() {
        let address = "192.0.2.1:443".parse().unwrap();
        let setup: Arc<dyn McpHttpNetworkAuthorizer> = authority(None, true);
        for scope in [
            authority(Some("192.0.2.2:443".parse().unwrap()), true),
            authority(Some(address), false),
        ] {
            let lifetime = WorkerCancellation::new();
            let peer = LegacyAppPeer::new(address, true, lifetime.clone());
            assert!(matches!(
                peer.authorize_send(TARGET, &setup, &message(1, scope))
                    .await,
                Err(McpError::PermissionDenied)
            ));
            assert!(lifetime.is_cancelled());
            assert!(matches!(
                peer.authorize_send(TARGET, &setup, &message(2, authority(Some(address), true)))
                    .await,
                Err(McpError::PermissionDenied)
            ));
        }
        let peer = LegacyAppPeer::new(address, true, WorkerCancellation::new());
        let options = peer
            .authorize_send(TARGET, &setup, &message(2, authority(Some(address), true)))
            .await
            .unwrap();
        assert_eq!(options.pinned_addresses, [address]);
    }

    #[tokio::test]
    async fn legacy_app_peer_correlates_a_callback_with_one_uncancelled_operation() {
        let address = "192.0.2.1:443".parse().unwrap();
        let peer = LegacyAppPeer::new(address, false, WorkerCancellation::new());
        let setup: Arc<dyn McpHttpNetworkAuthorizer> = authority(None, false);
        let caller = authority(Some(address), false);
        let callback = || {
            serde_json::from_value::<ServerJsonRpcMessage>(json!({
                "jsonrpc":"2.0","id":"callback","method":"roots/list"
            }))
            .unwrap()
        };
        assert!(!peer.receive(&mut callback()));
        peer.authorize_send(TARGET, &setup, &message(1, caller.clone()))
            .await
            .unwrap();
        let mut incoming = callback();
        assert!(peer.receive(&mut incoming));
        let ServerJsonRpcMessage::Request(request) = incoming else {
            panic!("roots request")
        };
        assert!(matches!(
            request.request.extensions().get::<InboundStreamOrigin>(),
            Some(InboundStreamOrigin::OutboundRequest(_))
        ));
        let reply =
            serde_json::from_value(json!({"jsonrpc":"2.0","id":"callback","result":{"roots":[]}}))
                .unwrap();
        assert!(peer.authorize_send(TARGET, &setup, &reply).await.is_ok());
        assert!(peer.authorize_send(TARGET, &setup, &reply).await.is_err());
        // Two outstanding requests cannot be correlated on these transports.
        peer.authorize_send(TARGET, &setup, &message(2, caller.clone()))
            .await
            .unwrap();
        assert!(!peer.receive(&mut callback()));
        caller.cancellation.cancel();
        assert!(!peer.receive(&mut callback()));
    }
}
