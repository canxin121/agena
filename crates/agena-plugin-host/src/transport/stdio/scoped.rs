//! One bootstrap process plus independently authorized session process instances.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use agena_process::sandbox::SandboxPolicy;
use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use super::StdioTransport;
use crate::config::{ConfiguredPlugin, PluginPackage, RestartMode, RestartPolicy};
use crate::effect_scope::PluginEffectScope;
use crate::error::TransportError;
use crate::host::{HostHandle, current_plugin_call_completion};
use crate::sdk::host_api::{
    HostPreparedProcessSandbox, HostProcessResources, current_host_callback_context,
};
use crate::sdk::rpc::method;
use crate::sdk::{PluginError, PluginErrorKind, PluginKey, ToolInvokeInput};
use crate::transport::initialization::PluginInitialization;
use crate::transport::{PluginTransport, ToolStreamHandle};

const MAX_SCOPED_PROCESSES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ProcessKey {
    scope: String,
    workspace_root: String,
    cwd: PathBuf,
    policy: SandboxPolicy,
    completion: Option<String>,
}

impl ProcessKey {
    fn from_prepared(
        prepared: HostPreparedProcessSandbox,
        runtime_paths: &[PathBuf],
        completion: Option<&crate::host::PluginCallCompletion>,
    ) -> Result<Self, TransportError> {
        if prepared.scope.trim().is_empty() || !Path::new(&prepared.cwd).is_absolute() {
            return Err(denied(
                "host returned an invalid process sandbox scope or cwd",
            ));
        }
        let mut policy: SandboxPolicy = serde_json::from_value(prepared.policy)?;
        policy.runtime_paths.extend_from_slice(runtime_paths);
        policy.runtime_paths.sort();
        policy.runtime_paths.dedup();
        let completion = if prepared.call_bound {
            Some(
                completion
                    .ok_or_else(|| {
                        denied("one-call process permission requires a live call completion")
                    })?
                    .id()
                    .to_owned(),
            )
        } else {
            None
        };
        Ok(Self {
            scope: prepared.scope,
            workspace_root: prepared.workspace_root,
            cwd: PathBuf::from(prepared.cwd),
            policy,
            completion,
        })
    }
}

struct LaunchRecipe {
    plugin_id: PluginKey,
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    restart: RestartPolicy,
    sha256: Option<String>,
    resources: HostProcessResources,
    runtime_paths: Vec<PathBuf>,
}

#[derive(Clone)]
struct Binding {
    host: Weak<HostHandle>,
    owner: Arc<PluginEffectScope>,
    initialization: Option<PluginInitialization>,
}

struct Inner {
    recipe: LaunchRecipe,
    bootstrap: Arc<StdioTransport>,
    _bootstrap_directory: tempfile::TempDir,
    binding: RwLock<Option<Binding>>,
    processes: Mutex<HashMap<ProcessKey, Arc<StdioTransport>>>,
    startup: Mutex<()>,
    stop: CancellationToken,
    closed: AtomicBool,
}

pub(crate) struct ScopedStdioTransport {
    inner: Arc<Inner>,
}

impl ScopedStdioTransport {
    pub(crate) async fn spawn(
        plugin_id: &str,
        configured: &ConfiguredPlugin,
        workspace_root: &Path,
        host: &Arc<HostHandle>,
    ) -> Result<Self, TransportError> {
        let plugin_id = plugin_id
            .parse()
            .map_err(|error| TransportError::Io(format!("invalid plugin id: {error}")))?;
        let configured = configured.clone();
        let workspace_root = workspace_root.to_owned();
        let (recipe, directory) = super::SANDBOX_WORKERS
            .run(move || prepare_recipe(plugin_id, configured, &workspace_root))
            .await
            .map_err(|error| {
                TransportError::Io(format!("plugin process preparation failed: {error}"))
            })??;
        let cwd = directory.path().to_owned();
        let policy = SandboxPolicy {
            disabled: !agena_process::sandbox::enabled(),
            readonly_paths: vec![cwd.clone()],
            runtime_paths: recipe.runtime_paths.clone(),
            ..Default::default()
        };
        let bootstrap = Arc::new(
            StdioTransport::spawn_with_sandbox_and_status(
                &recipe.command,
                &recipe.args,
                &recipe.env,
                Some(&cwd),
                None,
                recipe.restart.clone(),
                Some(recipe.plugin_id.clone()),
                Some(host.status_registry()),
                Some(host.log_store()),
                recipe.sha256.as_deref(),
                &policy,
            )
            .await?,
        );
        Ok(Self {
            inner: Arc::new(Inner {
                recipe,
                bootstrap,
                _bootstrap_directory: directory,
                binding: RwLock::new(None),
                processes: Mutex::new(HashMap::new()),
                startup: Mutex::new(()),
                stop: CancellationToken::new(),
                closed: AtomicBool::new(false),
            }),
        })
    }

    async fn process(&self) -> Result<Arc<StdioTransport>, TransportError> {
        let work = self.inner.process_for_current_call();
        tokio::select! {
            biased;
            _ = self.inner.stop.cancelled() => Err(TransportError::Cancelled),
            result = work => result,
        }
    }
}

impl Inner {
    async fn process_for_current_call(
        self: &Arc<Self>,
    ) -> Result<Arc<StdioTransport>, TransportError> {
        let context = current_host_callback_context()
            .filter(|context| context.authority_token.is_some())
            .ok_or_else(|| denied("stdio plugin execution requires an active host-issued scope"))?;
        let completion = current_plugin_call_completion();
        let binding = self
            .binding
            .read()
            .await
            .clone()
            .ok_or_else(|| denied("stdio plugin has no bound host"))?;
        let host = binding
            .host
            .upgrade()
            .ok_or_else(|| TransportError::disconnected("stdio plugin host was dropped"))?;
        if !host.is_current_effect_scope(&self.recipe.plugin_id, &binding.owner)
            || !binding.owner.is_accepting()
        {
            return Err(denied("stdio plugin belongs to a stale host generation"));
        }
        let initialization = binding
            .initialization
            .clone()
            .ok_or_else(|| denied("stdio plugin has not completed bootstrap initialization"))?;
        // The native host API validates the current callback authority and
        // projects its effective principal, ceilings and persisted rules.
        // No plugin RPC can supply this prepared policy.
        let client = host
            .scoped_host_client_for_scope(self.recipe.plugin_id.to_string(), binding.owner.clone());
        let prepared = client
            .prepare_process_sandbox(self.recipe.resources.clone())
            .await
            .map_err(TransportError::Plugin)?;
        let key =
            ProcessKey::from_prepared(prepared, &self.recipe.runtime_paths, completion.as_ref())?;
        if let Some(process) = self.processes.lock().await.get(&key).cloned() {
            if process.is_running().await {
                return Ok(process);
            }
        }

        // Serialize startup and disposal. Existing instances remain concurrent;
        // all permission checks happened before even consulting the cache.
        let _startup = self.startup.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Err(TransportError::disconnected("stdio plugin is closing"));
        }
        if let Some(process) = self.processes.lock().await.get(&key).cloned() {
            if process.is_running().await {
                return Ok(process);
            }
        }
        let obsolete = {
            let mut processes = self.processes.lock().await;
            let keys = processes
                .keys()
                .filter(|existing| {
                    **existing == key
                        || (existing.scope == key.scope
                            && existing.workspace_root == key.workspace_root
                            && existing.completion.is_none()
                            && key.completion.is_none())
                })
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| processes.remove(&key))
                .collect::<Vec<_>>()
        };
        close_processes(obsolete).await?;
        if self.processes.lock().await.len() >= MAX_SCOPED_PROCESSES {
            return Err(denied(
                "stdio plugin reached its active sandbox process limit",
            ));
        }

        // A scoped process never restarts in the background with a stale
        // approval. A later admitted call rechecks policy before replacing it.
        let mut restart = self.recipe.restart.clone();
        restart.policy = RestartMode::Never;
        let process = Arc::new(
            StdioTransport::spawn_with_sandbox_and_status(
                &self.recipe.command,
                &self.recipe.args,
                &self.recipe.env,
                Some(&key.cwd),
                None,
                restart,
                Some(self.recipe.plugin_id.clone()),
                None,
                Some(host.log_store()),
                self.recipe.sha256.as_deref(),
                &key.policy,
            )
            .await?,
        );
        let mut pending = PendingProcess(Some(process.clone()));
        process.restrict_callback_scope(&context, key.completion.is_some())?;
        process.bind_host(host, binding.owner).await?;
        let manifest = process
            .dispatch(method::META_MANIFEST, serde_json::json!({}))
            .await?;
        let manifest: crate::sdk::PluginManifest = serde_json::from_value(manifest)?;
        if manifest != initialization.manifest {
            return Err(denied(
                "scoped plugin manifest differs from its verified bootstrap manifest",
            ));
        }
        let mut scoped_initialization = initialization;
        scoped_initialization.workspace_root = PathBuf::from(&key.workspace_root);
        scoped_initialization
            .initialize_in_context(process.as_ref(), context)
            .await?;
        self.processes
            .lock()
            .await
            .insert(key.clone(), process.clone());
        pending.0.take();

        if key.completion.is_some() {
            let completion = completion.ok_or_else(|| denied("originating plugin call ended"))?;
            let owner = Arc::downgrade(self);
            let process = process.clone();
            tokio::spawn(async move {
                completion.wait().await;
                if let Some(owner) = owner.upgrade() {
                    let mut processes = owner.processes.lock().await;
                    if processes
                        .get(&key)
                        .is_some_and(|current| Arc::ptr_eq(current, &process))
                    {
                        processes.remove(&key);
                    }
                }
                if let Err(error) = process.close().await {
                    tracing::warn!(%error, "could not close one-call stdio sandbox");
                }
            });
        }
        Ok(process)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.closed.store(true, Ordering::Release);
        self.stop.cancel();
        let _startup = self.startup.lock().await;
        let processes = self
            .processes
            .lock()
            .await
            .drain()
            .map(|(_, process)| process)
            .collect::<Vec<_>>();
        let children = close_processes(processes).await;
        let bootstrap = self.bootstrap.close().await;
        children.and(bootstrap)
    }
}

struct PendingProcess(Option<Arc<StdioTransport>>);

impl Drop for PendingProcess {
    fn drop(&mut self) {
        if let Some(process) = self.0.take() {
            tokio::spawn(async move {
                if let Err(error) = process.close().await {
                    tracing::warn!(%error, "could not close incomplete scoped stdio process");
                }
            });
        }
    }
}

async fn close_processes(processes: Vec<Arc<StdioTransport>>) -> Result<(), TransportError> {
    let results = futures_util::future::join_all(
        processes
            .into_iter()
            .map(|process| async move { process.close().await }),
    )
    .await;
    let errors = results
        .into_iter()
        .filter_map(Result::err)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(TransportError::Io(errors.join("; ")))
    }
}

#[async_trait]
impl PluginTransport for ScopedStdioTransport {
    async fn bind_host(
        &self,
        host: Arc<HostHandle>,
        owner: Arc<PluginEffectScope>,
    ) -> Result<(), TransportError> {
        if self.inner.binding.read().await.is_some() {
            return Err(denied(
                "scoped stdio transport cannot move between host generations",
            ));
        }
        self.inner
            .bootstrap
            .bind_host(host.clone(), owner.clone())
            .await?;
        *self.inner.binding.write().await = Some(Binding {
            host: Arc::downgrade(&host),
            owner,
            initialization: None,
        });
        Ok(())
    }

    async fn initialize(
        &self,
        initialization: PluginInitialization,
    ) -> Result<crate::sdk::InitOutcome, TransportError> {
        let outcome = self
            .inner
            .bootstrap
            .initialize(initialization.clone())
            .await?;
        let mut binding = self.inner.binding.write().await;
        let binding = binding
            .as_mut()
            .ok_or_else(|| denied("stdio plugin initialization requires a bound host"))?;
        binding.initialization = Some(initialization);
        Ok(outcome)
    }

    async fn can_reuse(&self, _owner: &Arc<PluginEffectScope>) -> bool {
        // Reload must rebuild every policy-bound process and callback owner.
        false
    }

    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        match method {
            method::META_MANIFEST | method::META_INIT | method::META_SHUTDOWN => {
                self.inner.bootstrap.dispatch(method, params).await
            }
            _ => self.process().await?.dispatch(method, params).await,
        }
    }

    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), TransportError> {
        // Keep the call and any one-call sandbox alive through acknowledgment.
        self.dispatch(method, params).await.map(|_| ())
    }

    async fn invoke_stream(
        &self,
        input: ToolInvokeInput,
    ) -> Result<Option<ToolStreamHandle>, TransportError> {
        self.process().await?.invoke_stream(input).await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

impl Drop for ScopedStdioTransport {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.stop.cancel();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let inner = self.inner.clone();
            runtime.spawn(async move {
                if let Err(error) = inner.close().await {
                    tracing::warn!(%error, "could not close dropped scoped stdio transport");
                }
            });
        }
    }
}

fn prepare_recipe(
    plugin_id: PluginKey,
    configured: ConfiguredPlugin,
    workspace: &Path,
) -> Result<(LaunchRecipe, tempfile::TempDir), TransportError> {
    let PluginPackage::Stdio {
        command,
        mut args,
        env,
        cwd,
        reads,
        writes,
        network,
        restart,
        sha256,
    } = configured.package
    else {
        return Err(TransportError::Io(
            "scoped stdio requires a stdio package".into(),
        ));
    };
    let package_root = configured
        .package_root
        .map(|root| {
            if root.is_absolute() {
                root
            } else {
                workspace.join(root)
            }
        })
        .map(std::fs::canonicalize)
        .transpose()?;
    let base = package_root.as_deref().unwrap_or(workspace);
    let path = env
        .get("PATH")
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"));
    let executable = which::which_in(&command, path.as_deref(), base).map_err(|error| {
        TransportError::Io(format!("cannot resolve stdio plugin executable: {error}"))
    })?;
    let mut runtime_paths = package_root.iter().cloned().collect::<Vec<_>>();
    runtime_paths.extend(configured_entrypoint(&executable, &mut args, base)?);
    let directory = tempfile::Builder::new()
        .prefix("agena-plugin-bootstrap-")
        .tempdir()?;
    let command = executable
        .to_str()
        .ok_or_else(|| TransportError::Io("stdio executable path is not UTF-8".into()))?
        .to_owned();
    Ok((
        LaunchRecipe {
            plugin_id,
            command,
            args,
            env: env.into_iter().collect(),
            restart,
            sha256,
            resources: HostProcessResources {
                cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
                reads,
                writes,
                network,
            },
            runtime_paths,
        },
        directory,
    ))
}

/// Only interpreter entry points from installed configuration become code
/// grants. Data arguments and arbitrary argument parent directories do not.
fn configured_entrypoint(
    executable: &Path,
    arguments: &mut [String],
    base: &Path,
) -> Result<Vec<PathBuf>, TransportError> {
    let program = executable
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let python = program.starts_with("python");
    let node = matches!(program.as_str(), "node" | "nodejs");
    if !python && !node {
        return Ok(Vec::new());
    }
    for index in 0..arguments.len() {
        let argument = &arguments[index];
        if matches!(
            argument.as_str(),
            "-c" | "-m" | "-e" | "--eval" | "-p" | "--print"
        ) {
            return Ok(Vec::new());
        }
        if argument == "--" {
            continue;
        }
        if argument.starts_with('-') {
            if matches!(
                argument.as_str(),
                "-I" | "-B"
                    | "-E"
                    | "-s"
                    | "-S"
                    | "-u"
                    | "-O"
                    | "-OO"
                    | "--no-warnings"
                    | "--enable-source-maps"
            ) {
                continue;
            }
            // Unknown flag operands must not be mistaken for executable code.
            return Ok(Vec::new());
        }
        let entry = std::fs::canonicalize(base.join(argument))?;
        arguments[index] = entry
            .to_str()
            .ok_or_else(|| TransportError::Io("stdio entry point path is not UTF-8".into()))?
            .to_owned();
        let mut paths = vec![entry.clone()];
        if let Some(packages) = entry.ancestors().find(|path| {
            path.file_name().is_some_and(|name| {
                name == "node_modules" || name == "site-packages" || name == "dist-packages"
            })
        }) {
            paths.push(packages.to_owned());
        }
        return Ok(paths);
    }
    Ok(Vec::new())
}

fn denied(message: &str) -> TransportError {
    TransportError::Plugin(PluginError::from_kind(
        PluginErrorKind::PolicyDenied,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(scope: &str, policy: SandboxPolicy) -> HostPreparedProcessSandbox {
        let workspace = std::env::temp_dir().to_string_lossy().into_owned();
        HostPreparedProcessSandbox {
            scope: scope.into(),
            workspace_root: workspace.clone(),
            cwd: workspace,
            policy: serde_json::to_value(policy).unwrap(),
            call_bound: false,
        }
    }

    #[test]
    fn scoped_process_keys_separate_sessions_and_complete_permissions() {
        let first =
            ProcessKey::from_prepared(prepared("session:1", SandboxPolicy::default()), &[], None)
                .unwrap();
        let second =
            ProcessKey::from_prepared(prepared("session:2", SandboxPolicy::default()), &[], None)
                .unwrap();
        assert_ne!(first, second);
        let restricted = SandboxPolicy {
            denied_paths: vec![PathBuf::from("/workspace/private")],
            ..Default::default()
        };
        let third =
            ProcessKey::from_prepared(prepared("session:1", restricted), &[], None).unwrap();
        assert_ne!(first, third);
        let mut cache = HashMap::new();
        cache.insert(first, "first");
        assert!(!cache.contains_key(&second));
        assert!(!cache.contains_key(&third));
    }

    #[test]
    fn one_call_sandbox_cannot_start_without_its_completion() {
        let mut prepared = prepared("operator:call:1", SandboxPolicy::default());
        prepared.call_bound = true;
        assert!(ProcessKey::from_prepared(prepared, &[], None).is_err());
    }

    #[test]
    fn interpreter_data_arguments_do_not_become_runtime_grants() {
        let mut arguments = vec!["-c".into(), "print(1)".into(), "/secret/data".into()];
        assert!(
            configured_entrypoint(Path::new("python3"), &mut arguments, Path::new("/"))
                .unwrap()
                .is_empty()
        );
        let mut arguments = vec!["--require".into(), "/secret/data".into(), "entry.js".into()];
        assert!(
            configured_entrypoint(Path::new("node"), &mut arguments, Path::new("/"))
                .unwrap()
                .is_empty()
        );
    }
}
