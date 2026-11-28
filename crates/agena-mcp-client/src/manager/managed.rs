use std::sync::{Mutex as StdMutex, atomic::AtomicBool};

use serde_json::{Value, json};

use super::*;

pub struct McpManagedConnection {
    workspace: PathBuf,
    lifetime: CancellationToken,
    finished: CancellationToken,
    first_attempt: CancellationToken,
    close_confirmed: AtomicBool,
    status: StdMutex<Value>,
    transport: StdMutex<Option<Arc<McpConnectionManager>>>,
    observations: watch::Sender<u64>,
}

impl McpManagedConnection {
    pub fn cancellation(&self) -> CancellationToken {
        self.lifetime.clone()
    }

    pub fn status(&self) -> Value {
        self.status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn publish(&self, status: Value) {
        let settled = status["state"] != "connecting";
        let mut current = self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *current != status {
            *current = status;
            self.observations.send_modify(|revision| *revision += 1);
        }
        if settled {
            self.first_attempt.cancel();
        }
    }

    pub fn set_transport(&self, transport: Option<Arc<McpConnectionManager>>) {
        *self
            .transport
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = transport;
    }

    pub fn finish(&self, close_confirmed: bool) {
        self.first_attempt.cancel();
        self.close_confirmed
            .store(close_confirmed, Ordering::Release);
        self.finished.cancel();
    }

    pub async fn wait_closed(&self) -> bool {
        self.finished.cancelled().await;
        self.close_confirmed.load(Ordering::Acquire)
    }

    pub fn close_result(&self) -> Option<bool> {
        self.finished
            .is_cancelled()
            .then(|| self.close_confirmed.load(Ordering::Acquire))
    }
}

impl McpConnectionManager {
    pub async fn replace_managed_connection(
        &self,
        session: i64,
        name: &str,
        workspace: &Path,
        lifetime: CancellationToken,
    ) -> McpResult<(Arc<McpManagedConnection>, Option<Arc<McpManagedConnection>>)> {
        self.begin_managed_connection(session, name, workspace, lifetime, true)
            .await?
            .ok_or(McpError::Shutdown)
    }

    /// Admit the first ordinary call that can use the owning session's grant.
    /// A peer in backoff or with a spent retry budget keeps its existing slot.
    pub async fn reserve_managed_connection(
        &self,
        session: i64,
        name: &str,
        workspace: &Path,
        lifetime: CancellationToken,
    ) -> McpResult<Option<Arc<McpManagedConnection>>> {
        Ok(self
            .begin_managed_connection(session, name, workspace, lifetime, false)
            .await?
            .map(|(current, _)| current))
    }

    async fn begin_managed_connection(
        &self,
        session: i64,
        name: &str,
        workspace: &Path,
        lifetime: CancellationToken,
        replace: bool,
    ) -> McpResult<Option<(Arc<McpManagedConnection>, Option<Arc<McpManagedConnection>>)>> {
        let _lifecycle = self.lifecycle.read().await;
        if lifetime.is_cancelled() || !self.is_plugin_server(name).await {
            return Err(McpError::Shutdown);
        }
        let mut managed = self.managed_connections.lock().await;
        let key = (session, name.to_owned());
        if !replace && managed.contains_key(&key) {
            return Ok(None);
        }
        if managed.len() >= 128 && !managed.contains_key(&key) {
            return Err(McpError::Transport(
                "managed MCP connection limit reached".into(),
            ));
        }
        let current = Arc::new(McpManagedConnection {
            workspace: workspace.to_owned(),
            lifetime,
            finished: CancellationToken::new(),
            first_attempt: CancellationToken::new(),
            close_confirmed: AtomicBool::new(false),
            status: StdMutex::new(json!({ "state": "connecting", "attempt": 0 })),
            transport: StdMutex::new(None),
            observations: self.observations.clone(),
        });
        let old = managed.insert(key, current.clone());
        if let Some(old) = &old {
            old.lifetime.cancel();
        }
        self.notify_observation();
        Ok(Some((current, old)))
    }

    pub async fn borrow_managed_http(
        &self,
        session: i64,
        name: &str,
        workspace: &Path,
        authority: Arc<dyn crate::McpHttpNetworkAuthorizer>,
    ) -> McpResult<Arc<Self>> {
        let scope = self
            .managed_connections
            .lock()
            .await
            .get(&(session, name.to_owned()))
            .filter(|scope| scope.workspace == workspace)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))?;
        tokio::select! {
            biased;
            _ = scope.lifetime.cancelled() => return Err(McpError::Shutdown),
            _ = scope.first_attempt.cancelled() => {}
        }
        let transport = scope
            .transport
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_owned()))?;
        transport.borrow_http(name, authority, true).await
    }

    pub async fn managed_connection_status(
        &self,
        session: i64,
        name: &str,
        workspace: &Path,
    ) -> Option<Value> {
        self.managed_connections
            .lock()
            .await
            .get(&(session, name.to_owned()))
            .filter(|scope| scope.workspace == workspace)
            .map(|scope| scope.status())
    }

    pub(super) async fn retire_managed_connections(&self, remove: impl Fn(i64, &str) -> bool) {
        let mut managed = self.managed_connections.lock().await;
        let retired = managed
            .extract_if(.., |(session, name), _| remove(*session, name))
            .map(|(_, scope)| scope)
            .collect::<Vec<_>>();
        if !retired.is_empty() {
            for scope in retired {
                scope.lifetime.cancel();
            }
            self.notify_observation();
        }
    }

    /// Return whether a dedicated background connection confirmed closure.
    pub async fn close_managed_transport(&self, name: &str) -> bool {
        let _lifecycle = self.lifecycle.write().await;
        let server = self.inner.write().await.servers.remove(name);
        if let Some(server) = server {
            shutdown_server(server).await
        } else {
            true
        }
    }
}
