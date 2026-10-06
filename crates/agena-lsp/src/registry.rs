//! Multi-server registry. Maps a file path to the right LSP client and
//! lazily spawns the underlying server.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

use crate::client::LspClient;
use crate::error::{LspError, LspResult};
use crate::server_spec::LspServerSpec;
use crate::transport::StdioTransport;

static ROOT_RESOLVERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

#[derive(Debug, Error)]
/// Error resolving an LSP server.
pub enum ResolveError {
    #[error("no LSP server matches `{0}`")]
    NoServer(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClientKey {
    server: String,
    root: PathBuf,
}

/// File-system discovery, not a claim that a server can initialize successfully.
pub struct ServerStatus {
    pub spec: LspServerSpec,
    pub executable: Option<PathBuf>,
    pub command_available: Option<bool>,
    pub running_roots: Vec<PathBuf>,
}

/// Registry of LSP servers.
pub struct LspRegistry {
    workspace_root: PathBuf,
    client_name: String,
    client_version: String,
    servers: RwLock<BTreeMap<String, LspServerSpec>>,
    spawned: RwLock<HashMap<ClientKey, Arc<LspClient>>>,
    starting: Mutex<HashMap<ClientKey, Weak<Mutex<()>>>>,
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
            servers: RwLock::new(BTreeMap::new()),
            spawned: RwLock::new(HashMap::new()),
            starting: Mutex::new(HashMap::new()),
            lifecycle: RwLock::new(()),
        }
    }

    pub async fn register(&self, spec: LspServerSpec) {
        // Serialize configuration replacement with startup and shutdown. A
        // changed command/options must never reuse a client of the old spec.
        let _lifecycle_guard = self.lifecycle.write().await;
        let name = spec.name.clone();
        let old = self
            .servers
            .write()
            .await
            .insert(name.clone(), spec.clone());
        if old.as_ref().is_none_or(|old| old == &spec) {
            return;
        }
        let clients = {
            let mut spawned = self.spawned.write().await;
            let keys: Vec<_> = spawned
                .keys()
                .filter(|key| key.server == name)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| spawned.remove(&key))
                .collect()
        };
        shutdown_clients(clients).await;
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

    /// Inspect commands from the configured process PATH without starting them.
    /// Relative command/PATH entries are inspected from the workspace root;
    /// individual project roots can have different relative executables.
    pub async fn server_statuses(&self) -> LspResult<Vec<ServerStatus>> {
        let specs = self.server_specs().await;
        let running: Vec<_> = self.spawned.read().await.keys().cloned().collect();
        let workspace_root = self.workspace_root.clone();
        tokio::task::spawn_blocking(move || {
            specs
                .into_iter()
                .map(|spec| {
                    let path = spec
                        .env
                        .get("PATH")
                        .map(std::ffi::OsString::from)
                        .or_else(|| std::env::var_os("PATH"));
                    // which uses this process's PATHEXT on Windows. A per-server
                    // override makes that lookup inconclusive; do not guess.
                    let supported = !cfg!(windows) || !spec.env.contains_key("PATHEXT");
                    let executable = supported
                        .then(|| {
                            which::which_in(&spec.command, path.as_ref(), &workspace_root).ok()
                        })
                        .flatten();
                    let inspected = supported
                        && (executable.is_some()
                            || path.is_some()
                            || Path::new(&spec.command).components().count() > 1);
                    let mut running_roots: Vec<_> = running
                        .iter()
                        .filter(|key| key.server == spec.name)
                        .map(|key| key.root.clone())
                        .collect();
                    running_roots.sort();
                    ServerStatus {
                        command_available: inspected.then_some(executable.is_some()),
                        executable,
                        running_roots,
                        spec,
                    }
                })
                .collect()
        })
        .await
        .map_err(|error| LspError::transport_error(&error))
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

    /// Prefer an explicit extension match to a catch-all server. Ties are
    /// resolved by server name, independent of registration/HashMap order.
    pub async fn server_for_path(&self, path: &Path) -> Option<LspServerSpec> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let g = self.servers.read().await;
        g.values()
            .find(|s| !s.file_extensions.is_empty() && s.handles_extension(&ext))
            .or_else(|| g.values().find(|s| s.file_extensions.is_empty()))
            .cloned()
    }

    /// Get a client for the configured server and resolved project root, or
    /// spawn and initialize it. Panics-free: any spawn / initialize failure surfaces as
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
        let spec = self
            .servers
            .read()
            .await
            .get(server_name)
            .cloned()
            .ok_or_else(|| LspError::UnknownServer(server_name.to_string()))?;
        let root_spec = spec.clone();
        let hint_dir = hint_dir.to_owned();
        let workspace = self.workspace_root.clone();
        let root_dir = ROOT_RESOLVERS
            .run(move || {
                let root = root_spec.resolve_root(&hint_dir, &workspace);
                std::fs::canonicalize(&root).unwrap_or(root)
            })
            .await
            .map_err(|error| LspError::transport_error(&error))?;
        let key = ClientKey {
            server: server_name.to_owned(),
            root: root_dir.clone(),
        };
        if let Some(client) = self.spawned.read().await.get(&key).cloned() {
            return Ok(client);
        }
        // Only one task may perform the expensive spawn/initialize sequence
        // for a server/root pair. Without this keyed single-flight, concurrent first
        // requests launch duplicate children and overwrite all but one in the
        // registry, leaking their reader tasks and stdio pipes.
        let start_lock = {
            let mut starting = self.starting.lock().await;
            starting.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = starting.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                starting.insert(key.clone(), Arc::downgrade(&lock));
                lock
            }
        };
        let _start_guard = start_lock.lock().await;
        if let Some(client) = self.spawned.read().await.get(&key).cloned() {
            return Ok(client);
        }
        let env: HashMap<String, String> = spec
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
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
            if let Err(cleanup_error) = client.close_transport().await {
                tracing::warn!(
                    server = server_name,
                    diagnostic = %agena_failure::diagnostic::format_error_chain(&cleanup_error),
                    "LSP initialization failed and closing its transport also failed"
                );
            }
            return Err(error);
        }
        let mut spawned = self.spawned.write().await;
        spawned.insert(key, client.clone());
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
        shutdown_clients(clients).await;
    }
}

async fn shutdown_clients(clients: Vec<Arc<LspClient>>) {
    let mut shutdowns = tokio::task::JoinSet::new();
    for (index, client) in clients.into_iter().enumerate() {
        shutdowns.spawn(async move { (index, client.shutdown().await) });
    }
    while let Some(result) = shutdowns.join_next().await {
        match result {
            Ok((_, Ok(()))) => {}
            Ok((index, Err(error))) => tracing::warn!(
                client_index = index,
                diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                "failed to shut down an LSP client"
            ),
            Err(error) => tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                "an LSP shutdown task failed"
            ),
        }
    }
}

#[cfg(test)]
mod tests;
