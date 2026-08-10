//! Multi-server registry. Maps a file path to the right LSP client and
//! lazily spawns the underlying server.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

use crate::client::LspClient;
use crate::error::{LspError, LspResult};
use crate::server_spec::LspServerSpec;
use crate::transport::StdioTransport;

#[derive(Debug, Error)]
/// Error resolving an LSP server.
pub enum ResolveError {
    #[error("no LSP server matches `{0}`")]
    NoServer(String),
}

/// Registry of LSP servers.
pub struct LspRegistry {
    workspace_root: PathBuf,
    client_name: String,
    client_version: String,
    servers: RwLock<HashMap<String, LspServerSpec>>,
    spawned: RwLock<HashMap<String, Arc<LspClient>>>,
    starting: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    lifecycle: RwLock<()>,
}

impl LspRegistry {
    pub fn new(
        workspace_root: PathBuf,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        Self {
            workspace_root,
            client_name: client_name.into(),
            client_version: client_version.into(),
            servers: RwLock::new(HashMap::new()),
            spawned: RwLock::new(HashMap::new()),
            starting: Mutex::new(HashMap::new()),
            lifecycle: RwLock::new(()),
        }
    }

    pub async fn register(&self, spec: LspServerSpec) {
        let mut g = self.servers.write().await;
        g.insert(spec.name.clone(), spec);
    }

    pub async fn server_names(&self) -> Vec<String> {
        let g = self.servers.read().await;
        g.keys().cloned().collect()
    }

    /// Snapshot every registered server's metadata (name, command,
    /// file_extensions). Used by host APIs that surface the configured LSP
    /// fleet to plugins or operators.
    pub async fn server_specs(&self) -> Vec<LspServerSpec> {
        self.servers.read().await.values().cloned().collect()
    }

    /// Collect cached `(uri, diagnostics)` pairs from every already-spawned
    /// client. Diagnostics only appear here after a tool has touched the file
    /// — this is a read-only observability hook, not a forced indexing pass.
    pub async fn collect_diagnostics(&self) -> Vec<(String, Vec<lsp_types::Diagnostic>)> {
        let spawned: Vec<Arc<crate::client::LspClient>> =
            self.spawned.read().await.values().cloned().collect();
        let mut out = Vec::new();
        for client in spawned {
            for (uri, diagnostics) in client.diagnostics_snapshot() {
                if diagnostics.is_empty() {
                    continue;
                }
                out.push((uri, diagnostics));
            }
        }
        out
    }

    /// Find the first registered server whose `file_extensions` matches
    /// the given path. Returns `None` if no server claims the file.
    pub async fn server_for_path(&self, path: &Path) -> Option<LspServerSpec> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let g = self.servers.read().await;
        g.values().find(|s| s.handles_extension(&ext)).cloned()
    }

    /// Get an already-spawned client by name, or spawn one and initialize
    /// it. Panics-free: any spawn / initialize failure surfaces as
    /// [`LspError`].
    pub async fn client_for(
        &self,
        server_name: &str,
        hint_dir: &Path,
    ) -> LspResult<Arc<LspClient>> {
        // A shutdown takes the write side of this gate. Holding a read permit
        // makes spawn/initialize/insert one lifecycle transaction so shutdown
        // cannot drain the map and then have an in-flight initializer insert
        // a live child after shutdown has returned.
        let _lifecycle_guard = self.lifecycle.read().await;
        if let Some(client) = self.spawned.read().await.get(server_name).cloned() {
            return Ok(client);
        }
        // Only one task may perform the expensive spawn/initialize sequence
        // for a server. Without this keyed single-flight, concurrent first
        // requests launch duplicate children and overwrite all but one in the
        // registry, leaking their reader tasks and stdio pipes.
        let start_lock = {
            let mut starting = self.starting.lock().await;
            starting.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = starting.get(server_name).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                starting.insert(server_name.to_string(), Arc::downgrade(&lock));
                lock
            }
        };
        let _start_guard = start_lock.lock().await;
        if let Some(client) = self.spawned.read().await.get(server_name).cloned() {
            return Ok(client);
        }
        let spec = {
            let g = self.servers.read().await;
            g.get(server_name)
                .cloned()
                .ok_or_else(|| LspError::UnknownServer(server_name.to_string()))?
        };
        let env: HashMap<String, String> = spec
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let root_dir = spec.resolve_root(hint_dir, &self.workspace_root);
        let transport =
            StdioTransport::spawn(&spec.name, &spec.command, &spec.args, &env, Some(&root_dir))
                .await?;
        let client = LspClient::new(transport);
        let root_uri = url::Url::from_directory_path(&root_dir)
            .ok()
            .and_then(|u| u.as_str().parse::<lsp_types::Uri>().ok());
        if let Err(error) = client
            .initialize(
                root_uri,
                &self.client_name,
                &self.client_version,
                spec.initialization_options.clone(),
            )
            .await
        {
            let _ = client.close_transport().await;
            return Err(error);
        }
        let mut spawned = self.spawned.write().await;
        spawned.insert(server_name.to_string(), client.clone());
        Ok(client)
    }

    /// Convenience: spawn / fetch the right server for a file path.
    pub async fn client_for_path(&self, path: &Path) -> LspResult<Arc<LspClient>> {
        let spec = self
            .server_for_path(path)
            .await
            .ok_or_else(|| LspError::UnknownServer(path.display().to_string()))?;
        let parent = path.parent().unwrap_or(&self.workspace_root);
        self.client_for(&spec.name, parent).await
    }

    pub async fn shutdown_all(&self) {
        let _lifecycle_guard = self.lifecycle.write().await;
        // Do not hold the registry write lock while asking a client to shut
        // down. Shutdown performs transport I/O and can wait for the server
        // reader task; keeping this lock held would block every other LSP
        // lookup for the whole transport timeout and makes re-entrant
        // shutdown paths prone to deadlock.
        let clients = {
            let mut spawned = self.spawned.write().await;
            spawned
                .drain()
                .map(|(_, client)| client)
                .collect::<Vec<_>>()
        };
        let mut shutdowns = tokio::task::JoinSet::new();
        for client in clients {
            shutdowns.spawn(async move {
                let _ = client.shutdown().await;
            });
        }
        while shutdowns.join_next().await.is_some() {}
    }
}
