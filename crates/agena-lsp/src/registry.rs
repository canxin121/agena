//! Multi-server registry. Maps a file path to the right LSP client and
//! lazily spawns the underlying server.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use agena_process::sandbox::SandboxPolicy;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

use crate::client::{LspClient, LspClientOptions};
use crate::error::{LspError, LspResult};
use crate::server_spec::LspServerSpec;
use crate::transport::StdioTransport;

static ROOT_RESOLVERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

/// Called after resolving the real project root and before looking up a cached
/// process. The caller authorizes that root against its current session policy.
/// It runs on the blocking policy worker so permission checks may wait for an
/// approval without blocking Tokio's executor.
pub type LspSandboxPolicyBuilder =
    Arc<dyn Fn(&Path) -> std::io::Result<SandboxPolicy> + Send + Sync>;

/// An allowed persistent scope may reuse a server; None confines its lifetime
/// to the returned client handle, as required by one-invocation approval.
pub type LspScopedSandboxBuilder = Arc<
    dyn Fn(&Path, &[PathBuf]) -> std::io::Result<(SandboxPolicy, Option<String>)> + Send + Sync,
>;

fn registry_policy(builder: LspSandboxPolicyBuilder) -> LspScopedSandboxBuilder {
    Arc::new(move |root, reads| {
        let policy = builder(root)?;
        if !reads.iter().all(|path| {
            policy
                .readonly_paths
                .iter()
                .chain(&policy.readwrite_paths)
                .any(|grant| path.starts_with(grant))
        }) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "language server requires authorization for its plugin files",
            ));
        }
        Ok((policy, Some("registry".into())))
    })
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
    plugin_workspace: Option<PathBuf>,
    // Keep the complete policy as part of equality, rather than relying on a
    // lossy fingerprint. Different session capabilities never share a child.
    sandbox: SandboxPolicy,
}

pub struct PluginLspLaunch {
    pub spec: LspServerSpec,
    pub root: PathBuf,
    pub reads: Vec<PathBuf>,
    pub options: LspClientOptions,
}

pub type PluginLspResolver = Arc<dyn Fn(&Path) -> Result<PluginLspLaunch, String> + Send + Sync>;

#[derive(Clone)]
struct RegisteredServer {
    spec: LspServerSpec,
    plugin: Option<PluginLspResolver>,
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
    servers: RwLock<BTreeMap<String, RegisteredServer>>,
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
        self.register_server(spec, None).await;
    }

    pub async fn register_plugin(&self, spec: LspServerSpec, resolver: PluginLspResolver) {
        self.register_server(spec, Some(resolver)).await;
    }

    async fn register_server(&self, spec: LspServerSpec, plugin: Option<PluginLspResolver>) {
        // Serialize configuration replacement with startup and shutdown. A
        // changed command/options must never reuse a client of the old spec.
        let _lifecycle_guard = self.lifecycle.write().await;
        let name = spec.name.clone();
        let is_plugin = plugin.is_some();
        let old = self.servers.write().await.insert(
            name.clone(),
            RegisteredServer {
                spec: spec.clone(),
                plugin,
            },
        );
        if old
            .as_ref()
            .is_none_or(|old| !is_plugin && old.plugin.is_none() && old.spec == spec)
        {
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
        self.servers
            .read()
            .await
            .values()
            .map(|server| server.spec.clone())
            .collect()
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
            .map(|server| &server.spec)
            .find(|s| !s.file_extensions.is_empty() && s.handles_extension(&ext))
            .or_else(|| {
                g.values()
                    .map(|server| &server.spec)
                    .find(|s| s.file_extensions.is_empty())
            })
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
        self.client_for_with_fallback(
            server_name,
            hint_dir,
            &self.workspace_root,
            registry_policy(readonly_project_policy()),
        )
        .await
    }

    async fn client_for_with_fallback(
        &self,
        server_name: &str,
        hint_dir: &Path,
        fallback: &Path,
        policy_builder: LspScopedSandboxBuilder,
    ) -> LspResult<Arc<LspClient>> {
        // A shutdown takes the write side of this gate. Holding a read permit
        // makes spawn/initialize/insert one lifecycle transaction so shutdown
        // cannot drain the map and then have an in-flight initializer insert
        // a live child after shutdown has returned.
        let _lifecycle_guard = self.lifecycle.read().await;
        let server = self
            .servers
            .read()
            .await
            .get(server_name)
            .cloned()
            .ok_or_else(|| LspError::UnknownServer(server_name.to_string()))?;
        let hint_dir = hint_dir.to_owned();
        let fallback = fallback.to_owned();
        let (spec, options, root_dir, sandbox, cache_scope, plugin_workspace) = ROOT_RESOLVERS
            .run(move || {
                let plugin_workspace = server.plugin.as_ref().map(|_| fallback.clone());
                let (spec, options, root, reads) = if let Some(resolve) = server.plugin {
                    let launch = resolve(&fallback).map_err(std::io::Error::other)?;
                    (launch.spec, launch.options, launch.root, launch.reads)
                } else {
                    let root = server.spec.resolve_root(&hint_dir, &fallback);
                    (server.spec, LspClientOptions::default(), root, Vec::new())
                };
                let root = std::fs::canonicalize(&root).unwrap_or(root);
                let reads = reads
                    .into_iter()
                    .map(|path| path.canonicalize())
                    .collect::<std::io::Result<Vec<_>>>()?;
                let (mut sandbox, scope) = policy_builder(&root, &reads)?;
                if plugin_workspace.is_some() {
                    if sandbox.disabled || !agena_process::sandbox::enabled() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "plugin language servers require Agena process sandboxing",
                        ));
                    }
                } else {
                    sandbox
                        .runtime_paths
                        .extend(configured_runtime_paths(&spec, &root));
                }
                Ok::<_, std::io::Error>((spec, options, root, sandbox, scope, plugin_workspace))
            })
            .await
            .map_err(|error| LspError::transport_error(&error))??;
        let cacheable = cache_scope.is_some();
        let key = ClientKey {
            server: server_name.to_owned(),
            root: root_dir.clone(),
            scope: cache_scope.unwrap_or_default(),
            plugin_workspace,
            sandbox: sandbox.clone(),
        };
        if cacheable && let Some(client) = self.spawned.read().await.get(&key).cloned() {
            return Ok(client);
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
        if cacheable && let Some(client) = self.spawned.read().await.get(&key).cloned() {
            return Ok(client);
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
        .await?;
        let client = LspClient::with_options(transport, options);
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
        if cacheable {
            self.spawned.write().await.insert(key, client.clone());
        }
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
        if let Some(workspace_root) = workspace_root {
            let spec = self
                .server_for_path(file_path)
                .await
                .ok_or_else(|| LspError::UnknownServer(file_path.display().to_string()))?;
            let parent = file_path.parent().unwrap_or(workspace_root);
            return self
                .client_for_with_fallback(&spec.name, parent, workspace_root, policy_builder)
                .await;
        }
        if !file_path.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "LSP global target must be an absolute file path: {}",
                    file_path.display()
                ),
            )
            .into());
        }
        let path = file_path.to_owned();
        let parent = ROOT_RESOLVERS
            .run(move || global_file_parent(&path))
            .await
            .map_err(|error| LspError::transport_error(&error))??;
        let spec = self
            .server_for_path(file_path)
            .await
            .ok_or_else(|| LspError::UnknownServer(file_path.display().to_string()))?;
        self.client_for_with_fallback(&spec.name, &parent, &parent, policy_builder)
            .await
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
            initialization_options: None,
        }
    }

    struct IdleTransport;

    #[async_trait::async_trait]
    impl crate::transport::LspTransport for IdleTransport {
        async fn send(&self, _: serde_json::Value) -> LspResult<()> {
            Ok(())
        }

        async fn recv(&self) -> LspResult<crate::protocol::InboundMessage> {
            std::future::pending().await
        }

        async fn close(&self) -> LspResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn imported_lsp_authorizes_its_package_and_resolved_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let workspace = root.join("workspace");
        let package = root.join("plugin");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&package).unwrap();
        let registry = LspRegistry::new(root, "test", "0");
        let spec = test_spec("agena-test-missing-language-server", vec![]);
        let resolver_spec = spec.clone();
        let resolver_package = package.clone();
        registry
            .register_plugin(
                spec,
                Arc::new(move |workspace| {
                    Ok(PluginLspLaunch {
                        spec: resolver_spec.clone(),
                        root: workspace.to_path_buf(),
                        reads: vec![resolver_package.clone()],
                        options: Default::default(),
                    })
                }),
            )
            .await;
        let expected_workspace = workspace.clone();
        let error = registry
            .client_for_path_scoped(
                &workspace.join("test.txt"),
                Some(&workspace),
                Arc::new(move |root, reads| {
                    assert_eq!(root, expected_workspace);
                    assert_eq!(reads, std::slice::from_ref(&package));
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "plugin denied",
                    ))
                }),
            )
            .await
            .err()
            .unwrap();
        assert!(
            matches!(error, LspError::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );
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
        let cached = LspClient::new(Arc::new(IdleTransport));
        registry.spawned.write().await.insert(
            ClientKey {
                server: "sandbox-test".into(),
                root: project.clone(),
                scope: "registry".into(),
                plugin_workspace: None,
                sandbox: policy.clone(),
            },
            Arc::clone(&cached),
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
                        Arc::new(move |_, _| Ok((allowed.clone(), scope.clone()))),
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
            initialization_options: None,
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
