//! Multi-server registry. Maps a file path to the right LSP client and
//! lazily spawns the underlying server.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use agena_process::sandbox::SandboxPolicy;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tokio::time::Instant;

use crate::client::LspClient;
use crate::error::{LspError, LspResult};
use crate::server_spec::LspServerSpec;
use crate::transport::StdioTransport;

const MAX_CACHED_CLIENTS: usize = 4;
const CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

static ROOT_RESOLVERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

/// Called after resolving the real project root and before looking up a cached
/// process. The caller authorizes that root against its current session policy.
/// It runs on the blocking policy worker so permission checks may wait for an
/// approval without blocking Tokio's executor.
pub type LspSandboxPolicyBuilder =
    Arc<dyn Fn(&Path) -> std::io::Result<SandboxPolicy> + Send + Sync>;

/// An allowed persistent scope may reuse a server; None confines its lifetime
/// to the returned client handle, as required by one-invocation approval.
pub type LspScopedSandboxBuilder =
    Arc<dyn Fn(&Path) -> std::io::Result<(SandboxPolicy, Option<String>)> + Send + Sync>;

fn registry_policy(builder: LspSandboxPolicyBuilder) -> LspScopedSandboxBuilder {
    Arc::new(move |root| Ok((builder(root)?, Some("registry".into()))))
}

fn readonly_project_policy() -> LspSandboxPolicyBuilder {
    Arc::new(|root| {
        Ok(SandboxPolicy {
            readonly_paths: vec![root.to_path_buf()],
            ..Default::default()
        })
    })
}

/// Interpreter entry points come from trusted server configuration. Other
/// arguments may name project data, so they never become runtime grants.
fn configured_runtime_paths(spec: &LspServerSpec, root: &Path) -> Vec<PathBuf> {
    let program = Path::new(&spec.command)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let python = program.starts_with("python");
    let node = matches!(program.as_str(), "node" | "nodejs");
    if !python && !node {
        return Vec::new();
    }
    let mut arguments = spec.args.iter();
    let entry = loop {
        let Some(argument) = arguments.next() else {
            return Vec::new();
        };
        if matches!(
            argument.as_str(),
            "-c" | "-m" | "-e" | "--eval" | "-p" | "--print"
        ) || argument.starts_with("--eval=")
            || argument.starts_with("--print=")
        {
            return Vec::new();
        }
        if argument == "--" {
            break arguments.next();
        }
        if python && matches!(argument.as_str(), "-W" | "-X" | "--check-hash-based-pycs")
            || node
                && matches!(
                    argument.as_str(),
                    "-r" | "--require"
                        | "--import"
                        | "--loader"
                        | "--experimental-loader"
                        | "--conditions"
                        | "--input-type"
                )
        {
            if arguments.next().is_none() {
                return Vec::new();
            }
            continue;
        }
        if argument.starts_with('-') {
            let known_flag = if python {
                matches!(
                    argument.as_str(),
                    "-I" | "-B"
                        | "-E"
                        | "-s"
                        | "-S"
                        | "-u"
                        | "-O"
                        | "-OO"
                        | "-q"
                        | "-v"
                        | "-vv"
                        | "-d"
                        | "-b"
                        | "-bb"
                        | "-x"
                        | "-P"
                ) || argument.starts_with("-W")
                    || argument.starts_with("-X")
                    || argument.starts_with("--check-hash-based-pycs=")
            } else {
                matches!(
                    argument.as_str(),
                    "--no-warnings"
                        | "--trace-warnings"
                        | "--enable-source-maps"
                        | "--experimental-modules"
                ) || argument.contains('=')
            };
            if !known_flag {
                return Vec::new();
            }
            continue;
        }
        break Some(argument);
    };
    let Some(entry) = entry else {
        return Vec::new();
    };
    let path = root.join(entry);
    if !path.is_file() {
        return Vec::new();
    }
    let mut paths = vec![path.clone()];
    // Installed language-server packages import sibling modules. Restrict this
    // convenience grant to the conventional package directory, never HOME or
    // an arbitrary parent of a user-configured script.
    if let Some(packages) = path.ancestors().find(|path| {
        path.file_name().is_some_and(|name| {
            name == "node_modules" || name == "site-packages" || name == "dist-packages"
        })
    }) {
        paths.push(packages.to_path_buf());
    }
    paths
}

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
    scope: String,
    // Keep the complete policy as part of equality, rather than relying on a
    // lossy fingerprint. Different session capabilities never share a child.
    sandbox: SandboxPolicy,
}

/// File-system discovery, not a claim that a server can initialize successfully.
pub struct ServerStatus {
    pub spec: LspServerSpec,
    pub executable: Option<PathBuf>,
    pub command_available: Option<bool>,
    pub running_roots: Vec<PathBuf>,
}

/// How a caller may acquire a language-server process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LspClientMode {
    /// An explicit query may start a process for the duration of the call.
    Query,
    /// An explicit start must leave a process available to its owning session.
    Start,
    /// Automatic work can only borrow a process already running for this session.
    RunningOnly(i64),
}

/// An authorized connection and the route that selected it.
pub struct LspConnection {
    pub client: Arc<LspClient>,
    pub server: LspServerSpec,
    pub root: PathBuf,
}

struct CachedClient {
    client: Arc<LspClient>,
    last_used: Instant,
}

impl CachedClient {
    fn new(client: Arc<LspClient>) -> Self {
        Self {
            client,
            last_used: Instant::now(),
        }
    }

    fn has_callers(&self) -> bool {
        Arc::strong_count(&self.client) > 1
    }

    fn expired(&self) -> bool {
        !self.has_callers() && self.last_used.elapsed() >= CLIENT_IDLE_TIMEOUT
    }
}

/// Registry of LSP servers.
pub struct LspRegistry {
    workspace_root: PathBuf,
    client_name: String,
    client_version: String,
    servers: RwLock<BTreeMap<String, LspServerSpec>>,
    spawned: RwLock<HashMap<ClientKey, CachedClient>>,
    starting: Mutex<HashMap<ClientKey, Weak<Mutex<()>>>>,
    lifecycle: RwLock<()>,
}

impl LspRegistry {
    pub fn new(
        workspace_root: PathBuf,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        Self::with_servers(workspace_root, client_name, client_version, BTreeMap::new())
    }

    /// Publish the full routing table before any request can start a server.
    pub fn with_servers(
        workspace_root: PathBuf,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
        servers: BTreeMap<String, LspServerSpec>,
    ) -> Self {
        Self {
            workspace_root,
            client_name: client_name.into(),
            client_version: client_version.into(),
            servers: RwLock::new(servers),
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
                .filter_map(|key| spawned.remove(&key).map(|entry| entry.client))
                .collect()
        };
        let _ = shutdown_clients(clients).await;
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
        self.server_statuses_filtered(None).await
    }

    /// Return running roots owned by the requesting session.
    pub async fn server_statuses_for_session(
        &self,
        session: Option<i64>,
    ) -> LspResult<Vec<ServerStatus>> {
        let scope = session.map(session_scope).unwrap_or_default();
        self.server_statuses_filtered(Some(&scope)).await
    }

    async fn server_statuses_filtered(&self, scope: Option<&str>) -> LspResult<Vec<ServerStatus>> {
        let specs = self.server_specs().await;
        let running: Vec<_> = self
            .spawned
            .read()
            .await
            .iter()
            .filter(|(key, entry)| {
                entry.client.is_alive() && scope.is_none_or(|scope| key.scope == scope)
            })
            .map(|(key, _)| (key.server.clone(), key.root.clone()))
            .collect();
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
                        .filter(|(server, _)| server == &spec.name)
                        .map(|(_, root)| root.clone())
                        .collect();
                    running_roots.sort();
                    running_roots.dedup();
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
        let spawned: Vec<Arc<crate::client::LspClient>> = self
            .spawned
            .read()
            .await
            .values()
            .map(|entry| Arc::clone(&entry.client))
            .collect();
        let mut by_uri = BTreeMap::<String, Vec<lsp_types::Diagnostic>>::new();
        for client in spawned {
            for (uri, diagnostics) in client.diagnostics_snapshot() {
                let entries = by_uri.entry(uri).or_default();
                for diagnostic in diagnostics {
                    if !entries.contains(&diagnostic) {
                        entries.push(diagnostic);
                    }
                }
            }
        }
        by_uri.into_iter().collect()
    }

    /// Prefer explicit extensions to catch-all routes and custom servers to built-ins.
    pub async fn server_for_path(&self, path: &Path) -> Option<LspServerSpec> {
        self.server_for_path_named(path, None).await
    }

    pub async fn server_for_path_named(
        &self,
        path: &Path,
        name: Option<&str>,
    ) -> Option<LspServerSpec> {
        self.matching_servers(path, name).await.into_iter().next()
    }

    async fn matching_servers(&self, path: &Path, name: Option<&str>) -> Vec<LspServerSpec> {
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        let servers = self.servers.read().await;
        let mut matches = servers
            .values()
            .filter(|server| {
                name.is_none_or(|name| name == server.name) && server.handles_extension(ext)
            })
            .cloned()
            .collect::<Vec<_>>();
        matches.sort_by_key(|server| (server.file_extensions.is_empty(), server.built_in));
        matches
    }

    /// Get a client for the configured server and resolved project root, or
    /// spawn and initialize it. Panics-free: any spawn / initialize failure surfaces as
    /// [`LspError`].
    pub async fn client_for(
        &self,
        server_name: &str,
        hint_dir: &Path,
    ) -> LspResult<Arc<LspClient>> {
        let spec = self
            .servers
            .read()
            .await
            .get(server_name)
            .cloned()
            .ok_or_else(|| LspError::UnknownServer(server_name.to_owned()))?;
        self.client_for_with_fallback(
            &spec,
            hint_dir,
            Some(&self.workspace_root),
            registry_policy(readonly_project_policy()),
            LspClientMode::Query,
            None,
        )
        .await?
        .map(|(client, _)| client)
        .ok_or_else(|| LspError::TransportClosed("LSP connection unavailable".into()))
    }

    async fn client_for_with_fallback(
        &self,
        spec: &LspServerSpec,
        hint_dir: &Path,
        workspace_root: Option<&Path>,
        policy_builder: LspScopedSandboxBuilder,
        mode: LspClientMode,
        running_roots: Option<Vec<PathBuf>>,
    ) -> LspResult<Option<(Arc<LspClient>, PathBuf)>> {
        let server_name = &spec.name;
        let root_spec = spec.clone();
        let hint_dir = hint_dir.to_owned();
        let workspace_root = workspace_root.map(Path::to_owned);
        let resolved = ROOT_RESOLVERS
            .run(move || {
                let fallback = workspace_root.as_deref().unwrap_or(&hint_dir);
                let root = root_spec.resolve_root_with_boundary(
                    &hint_dir,
                    fallback,
                    workspace_root.as_deref(),
                );
                let root = std::fs::canonicalize(&root).unwrap_or(root);
                if running_roots
                    .as_ref()
                    .is_some_and(|roots| !roots.contains(&root))
                {
                    return Ok(None);
                }
                let (mut sandbox, scope) = policy_builder(&root)?;
                sandbox
                    .runtime_paths
                    .extend(configured_runtime_paths(&root_spec, &root));
                Ok::<_, std::io::Error>(Some((root, sandbox, scope)))
            })
            .await
            .map_err(|error| LspError::transport_error(&error))??;
        let Some((root_dir, sandbox, cache_scope)) = resolved else {
            return Ok(None);
        };
        // Root authorization runs before entering the startup/shutdown gate.
        let _lifecycle_guard = self.lifecycle.read().await;
        if self.servers.read().await.get(server_name) != Some(spec) {
            return Err(LspError::Protocol(format!(
                "LSP server `{server_name}` changed during lookup"
            )));
        }
        let cacheable = cache_scope.is_some();
        let key = ClientKey {
            server: server_name.to_owned(),
            root: root_dir.clone(),
            scope: cache_scope.unwrap_or_default(),
            sandbox: sandbox.clone(),
        };
        if let LspClientMode::RunningOnly(session) = mode {
            if key.scope != session_scope(session) {
                return Ok(None);
            }
            return Ok(self
                .cached_client(&key, false)
                .await
                .map(|client| (client, root_dir)));
        }
        if !cacheable && mode == LspClientMode::Start {
            return Err(LspError::PersistentAccessRequired);
        }
        if cacheable && let Some(client) = self.cached_client(&key, true).await {
            return Ok(Some((client, root_dir)));
        }
        // Only one task may perform the expensive spawn/initialize sequence
        // for a server/root pair. Without this keyed single-flight, concurrent first
        // requests launch duplicate children and overwrite all but one in the
        // registry, leaking their reader tasks and stdio pipes.
        let start_lock = if cacheable {
            let mut starting = self.starting.lock().await;
            starting.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = starting.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                starting.insert(key.clone(), Arc::downgrade(&lock));
                lock
            }
        } else {
            Arc::new(Mutex::new(()))
        };
        let _start_guard = start_lock.lock().await;
        if cacheable && let Some(client) = self.cached_client(&key, true).await {
            return Ok(Some((client, root_dir)));
        }
        if cacheable && let Some(dead) = self.spawned.write().await.remove(&key) {
            retire_clients(vec![dead.client]);
        }
        let env: HashMap<String, String> = spec
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let transport = StdioTransport::spawn(
            &spec.name,
            &spec.command,
            &spec.args,
            &env,
            Some(&root_dir),
            &sandbox,
        )
        .await
        .map_err(|error| {
            LspError::Transport(agena_failure::diagnostic::format_error_chain_with_context(
                format!("start LSP server `{server_name}` using `{}`", spec.command),
                &error,
            ))
        })?;
        let client = LspClient::new(transport);
        let root_uri = url::Url::from_directory_path(&root_dir)
            .ok()
            .and_then(|u| u.as_str().parse::<lsp_types::Uri>().ok());
        if let Err(error) = client
            .initialize_with_configuration(
                root_uri,
                &self.client_name,
                &self.client_version,
                spec.initialization_options.clone(),
                spec.configuration.clone(),
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
        if cacheable
            && !self.cache_client(key, Arc::clone(&client)).await
            && mode == LspClientMode::Start
        {
            // A query may use an uncached process when all cached clients are
            // busy. A start call promises an instance that can be reused.
            let _ = shutdown_clients(vec![client]).await;
            return Err(LspError::PoolBusy);
        }
        Ok(Some((client, root_dir)))
    }

    /// Convenience: spawn / fetch the right server for a file path.
    pub async fn client_for_path(&self, path: &Path) -> LspResult<Arc<LspClient>> {
        self.client_for_path_scoped(
            path,
            Some(&self.workspace_root),
            registry_policy(readonly_project_policy()),
        )
        .await
    }

    /// Resolve an explicitly selected absolute file without using the registry's
    /// workspace as a fallback. If no marker is found, use the file's parent.
    pub async fn client_for_path_global(&self, file_path: &Path) -> LspResult<Arc<LspClient>> {
        self.client_for_path_sandboxed(file_path, None, readonly_project_policy())
            .await
    }

    /// Resolve a file using project or global-session semantics, authorize the
    /// resolved root, and reuse only a process with the same sandbox policy.
    /// MXC creates the process's private writable home/cache; the supplied
    /// project grants can remain read-only.
    pub async fn client_for_path_sandboxed(
        &self,
        file_path: &Path,
        workspace_root: Option<&Path>,
        policy_builder: LspSandboxPolicyBuilder,
    ) -> LspResult<Arc<LspClient>> {
        self.client_for_path_scoped(file_path, workspace_root, registry_policy(policy_builder))
            .await
    }

    /// Session-owned clients must supply their actual authority scope. A None
    /// scope creates an uncached client that is removed when its caller drops it.
    pub async fn client_for_path_scoped(
        &self,
        file_path: &Path,
        workspace_root: Option<&Path>,
        policy_builder: LspScopedSandboxBuilder,
    ) -> LspResult<Arc<LspClient>> {
        self.client_for_path_scoped_with_server(file_path, workspace_root, None, policy_builder)
            .await
            .map(|(client, _)| client)
    }

    /// Return the client and the exact routing configuration that owns it.
    pub async fn client_for_path_scoped_with_server(
        &self,
        file_path: &Path,
        workspace_root: Option<&Path>,
        server: Option<&str>,
        policy_builder: LspScopedSandboxBuilder,
    ) -> LspResult<(Arc<LspClient>, LspServerSpec)> {
        self.connection_for_path(
            file_path,
            workspace_root,
            server,
            policy_builder,
            LspClientMode::Query,
        )
        .await?
        .map(|connection| (connection.client, connection.server))
        .ok_or_else(|| LspError::TransportClosed("LSP connection unavailable".into()))
    }

    /// Resolve a route and acquire a process under the caller's complete policy.
    pub async fn connection_for_path(
        &self,
        file_path: &Path,
        workspace_root: Option<&Path>,
        server: Option<&str>,
        policy_builder: LspScopedSandboxBuilder,
        mode: LspClientMode,
    ) -> LspResult<Option<LspConnection>> {
        let parent = if let Some(root) = workspace_root {
            file_path.parent().unwrap_or(root).to_owned()
        } else {
            let path = file_path.to_owned();
            ROOT_RESOLVERS
                .run(move || global_file_parent(&path))
                .await
                .map_err(|error| LspError::transport_error(&error))??
        };
        let servers = self.matching_servers(file_path, server).await;
        if let LspClientMode::RunningOnly(session) = mode {
            let scope = session_scope(session);
            let mut roots = BTreeMap::<String, Vec<PathBuf>>::new();
            for (key, entry) in self.spawned.read().await.iter() {
                if key.scope == scope && entry.client.is_alive() && !entry.expired() {
                    roots
                        .entry(key.server.clone())
                        .or_default()
                        .push(key.root.clone());
                }
            }
            for server in servers {
                if let Some(roots) = roots.remove(&server.name)
                    && let Some((client, root)) = self
                        .client_for_with_fallback(
                            &server,
                            &parent,
                            workspace_root,
                            Arc::clone(&policy_builder),
                            mode,
                            Some(roots),
                        )
                        .await?
                {
                    return Ok(Some(LspConnection {
                        client,
                        server,
                        root,
                    }));
                }
            }
            return Ok(None);
        }
        let server = servers.into_iter().next().ok_or_else(|| {
            LspError::UnknownServer(server.map_or_else(
                || file_path.display().to_string(),
                |name| format!("{name} for {}", file_path.display()),
            ))
        })?;
        Ok(self
            .client_for_with_fallback(&server, &parent, workspace_root, policy_builder, mode, None)
            .await?
            .map(|(client, root)| LspConnection {
                client,
                server,
                root,
            }))
    }

    /// Whether an automatic check might have a running instance to reuse.
    pub async fn has_running_session(&self, session: i64) -> bool {
        let scope = session_scope(session);
        self.spawned
            .read()
            .await
            .iter()
            .any(|(key, entry)| key.scope == scope && entry.client.is_alive() && !entry.expired())
    }

    async fn cached_client(&self, key: &ClientKey, touch: bool) -> Option<Arc<LspClient>> {
        let mut clients = self.spawned.write().await;
        let entry = clients
            .get_mut(key)
            .filter(|entry| entry.client.is_alive() && (touch || !entry.expired()))?;
        if touch {
            entry.last_used = Instant::now();
        }
        Some(Arc::clone(&entry.client))
    }

    async fn cache_client(&self, key: ClientKey, client: Arc<LspClient>) -> bool {
        let mut clients = self.spawned.write().await;
        let mut retired: Vec<_> = clients
            .extract_if(|_, entry| !entry.client.is_alive())
            .map(|(_, entry)| entry.client)
            .collect();
        if clients.len() >= MAX_CACHED_CLIENTS {
            let oldest = clients
                .iter()
                .filter(|(_, entry)| !entry.has_callers())
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest
                && let Some(entry) = clients.remove(&oldest)
            {
                retired.push(entry.client);
            }
        }
        let retained = clients.len() < MAX_CACHED_CLIENTS;
        if retained {
            clients.insert(key, CachedClient::new(client));
        }
        drop(clients);
        retire_clients(retired);
        retained
    }

    /// Drop unhealthy and idle clients without extending the lifetime of the registry.
    pub async fn maintain(registry: Weak<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let Some(registry) = registry.upgrade() else {
                return;
            };
            registry.reap_idle_clients().await;
        }
    }

    async fn reap_idle_clients(&self) {
        let expired = self
            .spawned
            .write()
            .await
            .extract_if(|_, entry| !entry.client.is_alive() || entry.expired())
            .map(|(_, entry)| entry.client)
            .collect();
        let _ = shutdown_clients(expired).await;
    }

    /// Release every client owned by a deleted session.
    pub async fn stop_session(&self, session: i64) {
        let _ = self.stop_session_servers(session, None).await;
    }

    /// Stop the caller's cached instances, optionally limited to one configured server.
    pub async fn stop_session_servers(
        &self,
        session: i64,
        server: Option<&str>,
    ) -> LspResult<usize> {
        let scope = session_scope(session);
        let _lifecycle = self.lifecycle.write().await;
        let clients: Vec<_> = self
            .spawned
            .write()
            .await
            .extract_if(|key, _| key.scope == scope && server.is_none_or(|name| key.server == name))
            .map(|(_, entry)| entry.client)
            .collect();
        let stopped = clients.iter().filter(|client| client.is_alive()).count();
        shutdown_clients(clients).await?;
        Ok(stopped)
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
                .map(|(_, entry)| entry.client)
                .collect::<Vec<_>>()
        };
        let _ = shutdown_clients(clients).await;
    }
}

fn session_scope(session: i64) -> String {
    format!("session:{session}")
}

fn global_file_parent(path: &Path) -> std::io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "LSP global target must be an absolute file path",
        ));
    }
    let canonical = std::fs::canonicalize(path)?;
    if !std::fs::metadata(&canonical)?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("LSP global target is not a file: {}", path.display()),
        ));
    }
    canonical.parent().map(Path::to_path_buf).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "LSP global target has no parent directory",
        )
    })
}

fn retire_clients(clients: Vec<Arc<LspClient>>) {
    if !clients.is_empty() {
        tokio::spawn(shutdown_clients(clients));
    }
}

async fn shutdown_clients(clients: Vec<Arc<LspClient>>) -> LspResult<()> {
    let mut shutdowns = tokio::task::JoinSet::new();
    for (index, client) in clients.into_iter().enumerate() {
        shutdowns.spawn(async move { (index, client.shutdown().await) });
    }
    let mut first_error = None;
    while let Some(result) = shutdowns.join_next().await {
        let error = match result {
            Ok((_, Ok(()))) => continue,
            Ok((index, Err(error))) => {
                tracing::warn!(
                    client_index = index,
                    diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                    "failed to shut down an LSP client"
                );
                error
            }
            Err(error) => {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                    "an LSP shutdown task failed"
                );
                LspError::transport_error(&error)
            }
        };
        first_error.get_or_insert(error);
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_spec(command: &str, args: Vec<String>) -> LspServerSpec {
        LspServerSpec {
            name: "sandbox-test".into(),
            command: command.into(),
            args,
            env: Default::default(),
            file_extensions: vec![],
            root_markers: vec![],
            ..Default::default()
        }
    }

    #[derive(Default)]
    struct IdleTransport {
        response: Mutex<Option<crate::protocol::InboundMessage>>,
        ready: tokio::sync::Notify,
    }

    #[async_trait::async_trait]
    impl crate::transport::LspTransport for IdleTransport {
        async fn send(&self, payload: serde_json::Value) -> LspResult<()> {
            if payload["method"] == "shutdown" {
                *self.response.lock().await = Some(crate::protocol::InboundMessage::from_value(
                    serde_json::json!({"jsonrpc": "2.0", "id": payload["id"], "result": null}),
                )?);
                self.ready.notify_one();
            }
            Ok(())
        }

        async fn recv(&self) -> LspResult<crate::protocol::InboundMessage> {
            self.ready.notified().await;
            Ok(self.response.lock().await.take().unwrap())
        }

        async fn close(&self) -> LspResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn current_session_policy_is_checked_before_reusing_a_cached_client() {
        let directory = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(directory.path()).unwrap();
        let file = project.join("sample.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let registry = LspRegistry::new(project.join("unrelated"), "test", "1");
        registry
            .register(test_spec("agena-test-missing-language-server", vec![]))
            .await;
        let policy = SandboxPolicy {
            readonly_paths: vec![project.clone()],
            ..Default::default()
        };
        let cached = LspClient::new(Arc::new(IdleTransport::default()));
        registry.spawned.write().await.insert(
            ClientKey {
                server: "sandbox-test".into(),
                root: project.clone(),
                scope: "registry".into(),
                sandbox: policy.clone(),
            },
            CachedClient::new(Arc::clone(&cached)),
        );
        let expected = project.clone();
        let allowed = policy.clone();
        let client = registry
            .client_for_path_sandboxed(
                &file,
                Some(&project),
                Arc::new(move |root| {
                    assert_eq!(root, expected);
                    Ok(allowed.clone())
                }),
            )
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&client, &cached));

        let error = registry
            .client_for_path_sandboxed(
                &file,
                Some(&project),
                Arc::new(|_| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "project access was revoked",
                    ))
                }),
            )
            .await
            .err()
            .unwrap();
        assert!(
            matches!(error, LspError::Io(ref error) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );

        for scope in [Some("session:7".to_string()), None] {
            let allowed = policy.clone();
            // Identical kernel grants cannot borrow a different session's
            // server or turn a one-call approval into a cached process.
            assert!(
                registry
                    .client_for_path_scoped(
                        &file,
                        Some(&project),
                        Arc::new(move |_| Ok((allowed.clone(), scope.clone()))),
                    )
                    .await
                    .is_err()
            );
        }
        let mut restricted = policy;
        restricted.denied_paths.push(project.join("secrets"));
        // A different policy must try to start its own child. The intentionally
        // missing executable makes that observable without starting a server.
        assert!(
            registry
                .client_for_path_sandboxed(
                    &file,
                    Some(&project),
                    Arc::new(move |_| Ok(restricted.clone())),
                )
                .await
                .is_err()
        );
        cached.close_transport().await.unwrap();
    }

    #[tokio::test]
    async fn specific_and_custom_routes_take_priority() {
        let rust = |name: &str, built_in| LspServerSpec {
            name: name.into(),
            file_extensions: vec!["rs".into()],
            built_in,
            ..Default::default()
        };
        let registry = LspRegistry::new(PathBuf::new(), "test", "1");
        registry.register(rust("a-built-in", true)).await;
        registry.register(rust("z-custom", false)).await;
        registry
            .register(LspServerSpec {
                name: "a-catch-all".into(),
                ..Default::default()
            })
            .await;
        let path = Path::new("main.RS");
        assert_eq!(
            registry.server_for_path(path).await.unwrap().name,
            "z-custom"
        );
        assert_eq!(
            registry
                .server_for_path_named(path, Some("a-built-in"))
                .await
                .unwrap()
                .name,
            "a-built-in"
        );
        assert_eq!(
            registry
                .server_for_path(Path::new("another.txt"))
                .await
                .unwrap()
                .name,
            "a-catch-all"
        );
    }

    #[tokio::test]
    async fn automatic_checks_only_borrow_an_exact_match_without_refreshing_idle_time() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let file = root.join("main.rs");
        let registry = LspRegistry::new(root.clone(), "test", "1");
        registry
            .register(test_spec("agena-test-missing-language-server", vec![]))
            .await;
        let missing_policy: LspScopedSandboxBuilder =
            Arc::new(|_| panic!("no project to authorize"));
        let mode = LspClientMode::RunningOnly(7);
        assert!(
            registry
                .connection_for_path(&file, Some(&root), None, missing_policy, mode)
                .await
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            registry
                .connection_for_path(
                    &file,
                    Some(&root),
                    None,
                    Arc::new(|_| Ok((SandboxPolicy::default(), None))),
                    LspClientMode::Start,
                )
                .await,
            Err(LspError::PersistentAccessRequired)
        ));

        let sandbox = SandboxPolicy {
            readonly_paths: vec![root.clone()],
            ..Default::default()
        };
        let key = ClientKey {
            server: "sandbox-test".into(),
            root: root.clone(),
            scope: session_scope(7),
            sandbox: sandbox.clone(),
        };
        let client = LspClient::new(Arc::new(IdleTransport::default()));
        let entry = CachedClient::new(Arc::clone(&client));
        let last_used = entry.last_used;
        registry.spawned.write().await.insert(key.clone(), entry);
        let exact: LspScopedSandboxBuilder =
            Arc::new(move |_| Ok((sandbox.clone(), Some(session_scope(7)))));
        let connection = registry
            .connection_for_path(&file, Some(&root), None, Arc::clone(&exact), mode)
            .await
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&connection.client, &client));
        assert_eq!(registry.spawned.read().await[&key].last_used, last_used);

        assert!(
            registry
                .connection_for_path(
                    &file,
                    Some(&root),
                    None,
                    exact,
                    LspClientMode::RunningOnly(8)
                )
                .await
                .unwrap()
                .is_none()
        );
        let broader = Arc::new(|_: &Path| Ok((SandboxPolicy::default(), Some(session_scope(7)))));
        assert!(
            registry
                .connection_for_path(&file, Some(&root), None, broader, mode)
                .await
                .unwrap()
                .is_none()
        );
        let nested = root.join("nested");
        std::fs::create_dir(&nested).unwrap();
        assert!(
            registry
                .connection_for_path(
                    &nested.join("main.rs"),
                    Some(&nested),
                    None,
                    Arc::new(|_| panic!("different root")),
                    mode,
                )
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn stop_filters_by_session_and_optional_server() {
        let registry = LspRegistry::new(PathBuf::new(), "test", "1");
        let mut tracked = Vec::new();
        for (session, server, root) in [
            (7, "a", "one"),
            (7, "a", "two"),
            (7, "b", "one"),
            (8, "a", "one"),
        ] {
            let client = LspClient::new(Arc::new(IdleTransport::default()));
            let key = ClientKey {
                server: server.into(),
                root: root.into(),
                scope: session_scope(session),
                sandbox: SandboxPolicy::default(),
            };
            registry
                .spawned
                .write()
                .await
                .insert(key, CachedClient::new(Arc::clone(&client)));
            tracked.push(client);
        }
        assert_eq!(
            registry.stop_session_servers(7, Some("a")).await.unwrap(),
            2
        );
        assert!(!tracked[0].is_alive() && !tracked[1].is_alive());
        assert!(tracked[2].is_alive() && tracked[3].is_alive());
        assert_eq!(registry.stop_session_servers(7, None).await.unwrap(), 1);
        assert!(!registry.has_running_session(7).await);
        assert!(registry.has_running_session(8).await);
        assert_eq!(registry.stop_session_servers(7, None).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn idle_reaping_preserves_borrowed_clients() {
        let registry = LspRegistry::new(PathBuf::new(), "test", "1");
        let idle = LspClient::new(Arc::new(IdleTransport::default()));
        let expired = Arc::downgrade(&idle);
        let borrowed = LspClient::new(Arc::new(IdleTransport::default()));
        for (root, client) in [("idle", idle), ("borrowed", Arc::clone(&borrowed))] {
            let key = ClientKey {
                server: "test".into(),
                root: root.into(),
                scope: session_scope(7),
                sandbox: SandboxPolicy::default(),
            };
            let mut entry = CachedClient::new(client);
            entry.last_used -= CLIENT_IDLE_TIMEOUT;
            registry.spawned.write().await.insert(key, entry);
        }

        registry.reap_idle_clients().await;
        assert!(expired.upgrade().is_none());
        assert!(borrowed.is_alive());
        assert_eq!(registry.spawned.read().await.len(), 1);
        drop(borrowed);
        registry.reap_idle_clients().await;
        assert!(registry.spawned.read().await.is_empty());
    }

    #[tokio::test]
    async fn cache_limit_leaves_busy_clients_in_place() {
        let registry = LspRegistry::new(PathBuf::new(), "test", "1");
        let key = |index: usize| ClientKey {
            server: "test".into(),
            root: index.to_string().into(),
            scope: session_scope(7),
            sandbox: SandboxPolicy::default(),
        };
        let mut borrowed = Vec::new();
        for index in 0..MAX_CACHED_CLIENTS {
            let client = LspClient::new(Arc::new(IdleTransport::default()));
            assert!(registry.cache_client(key(index), Arc::clone(&client)).await);
            borrowed.push(client);
        }
        let next = LspClient::new(Arc::new(IdleTransport::default()));
        let next_key = key(MAX_CACHED_CLIENTS);
        assert!(
            !registry
                .cache_client(next_key.clone(), Arc::clone(&next))
                .await
        );
        let reusable_slot = borrowed.len() - 1;
        drop(borrowed.pop());
        assert!(registry.cache_client(next_key.clone(), next).await);
        let cache = registry.spawned.read().await;
        assert_eq!(cache.len(), MAX_CACHED_CLIENTS);
        assert!(!cache.contains_key(&key(reusable_slot)));
        assert!(cache.contains_key(&next_key));
        assert!(borrowed.iter().all(|client| client.is_alive()));
    }

    #[test]
    fn interpreter_data_arguments_do_not_become_runtime_grants() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let entry = root.join("server.py");
        let data = root.join("private.json");
        std::fs::write(&entry, "").unwrap();
        std::fs::write(&data, "secret").unwrap();
        let spec = test_spec(
            "python3",
            vec!["-I".into(), "server.py".into(), "private.json".into()],
        );
        assert_eq!(configured_runtime_paths(&spec, root), vec![entry]);
        let inline = test_spec(
            "python3",
            vec!["-c".into(), "print('ok')".into(), "private.json".into()],
        );
        assert!(configured_runtime_paths(&inline, root).is_empty());
        let module = test_spec(
            "python3",
            vec!["-m".into(), "server".into(), "private.json".into()],
        );
        assert!(configured_runtime_paths(&module, root).is_empty());
        let node_inline = test_spec(
            "node",
            vec!["--eval=console.log('ok')".into(), "private.json".into()],
        );
        assert!(configured_runtime_paths(&node_inline, root).is_empty());
    }

    #[tokio::test]
    async fn global_file_root_never_falls_back_to_registry_workspace() {
        let dir = std::env::temp_dir().join(format!("agena-lsp-global-{}", std::process::id()));
        let selected = dir.join("selected");
        std::fs::create_dir_all(&selected).unwrap();
        let file = selected.join("example.rs");
        std::fs::write(&file, "").unwrap();
        let registry = LspRegistry::new(dir.join("unrelated-workspace"), "test", "1");
        let parent = global_file_parent(&file).unwrap();
        let spec = LspServerSpec {
            name: "test".into(),
            command: "test".into(),
            args: vec![],
            env: Default::default(),
            file_extensions: vec![],
            root_markers: vec!["missing-marker".into()],
            ..Default::default()
        };
        assert_eq!(spec.resolve_root(&parent, &parent), parent);
        assert_ne!(spec.resolve_root(&parent, &parent), registry.workspace_root);
        assert!(
            registry
                .client_for_path_global(Path::new("relative.rs"))
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("absolute")
        );
        assert!(
            registry
                .client_for_path_global(&selected)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("not a file")
        );
        assert!(
            registry
                .client_for_path_global(&selected.join("missing.rs"))
                .await
                .is_err()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
