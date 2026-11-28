mod assignments;
mod transcript;

use std::{
    collections::{BTreeMap, HashMap},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

use agena_domain::{AccessKind, PermissionDecision, SessionKind};
use agena_plugin_host::{
    PluginHost, PluginPackage,
    agent_plugin::FileWatches,
    sdk::{
        HookSubscription, PluginKey, SessionCwdChangedInput,
        host_api::{HookTranscriptFormat, ImportedHookEnvironment, ImportedHookFile},
    },
};
use agena_storage::kernel::{KernelReader, SessionId};
use tokio_util::sync::CancellationToken;

use super::ToolExecutor;
use crate::shell_sandbox::PrivateShellFile;
use transcript::SessionTranscripts;
pub use transcript::TranscriptError;

static ENVIRONMENT_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

fn private_directory() -> io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("agena-shell-session-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempdir()
}

pub(super) struct DirectoryCapture {
    file: tempfile::NamedTempFile,
    directory: tempfile::TempDir,
}

impl DirectoryCapture {
    pub(super) fn new() -> io::Result<Self> {
        let directory = private_directory()?;
        let file = tempfile::NamedTempFile::new_in(directory.path())?;
        Ok(Self { file, directory })
    }

    pub(super) fn argv(
        &self,
        launch: &agena_tool::shell::ShellLaunchSpec,
        command: &str,
    ) -> Option<Vec<String>> {
        let path = self.file.path().to_str()?;
        let pwd = if cfg!(windows) {
            "command pwd -W"
        } else {
            "command pwd -P"
        };
        let cleanup = format!(
            "agena_shell_status=$?\n{pwd} > {} || :\nexit \"$agena_shell_status\"",
            quote(path)
        );
        // The EXIT handler also covers explicit `exit` and errexit. Capture
        // after a normal return even if the command installed its own handler.
        Some(launch.argv(&format!(
            "trap {} 0\neval {}\n{cleanup}",
            quote(&cleanup),
            quote(command),
        )))
    }

    pub(super) fn resource(&self) -> PrivateShellFile {
        PrivateShellFile {
            directory: self.directory.path().to_path_buf(),
            path: self.file.path().to_path_buf(),
            writable: true,
        }
    }

    pub(super) fn into_cwd(self) -> Option<PathBuf> {
        // The file was opened before the process began. Reading the original
        // handle avoids following a new link or opening a special file.
        let mut value = String::new();
        self.file
            .as_file()
            .take(32 * 1024)
            .read_to_string(&mut value)
            .ok()?;
        let path = PathBuf::from(value.strip_suffix('\n')?);
        path.is_absolute().then_some(path)
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn source_environment(environment: &ImportedHookEnvironment, command: &str) -> Option<String> {
    let path = quote(environment.path().to_str()?);
    Some(format!("if [ -s {path} ]; then . {path}; fi\n{command}"))
}

pub(super) struct ImportedShellFiles {
    environment: Option<ImportedHookEnvironment>,
    transcripts: Vec<ImportedHookFile>,
}

impl ImportedShellFiles {
    pub(super) fn source(&self, command: &str) -> Option<String> {
        source_environment(self.environment.as_ref()?, command)
    }

    pub(super) fn readonly_mounts(&self) -> Vec<PrivateShellFile> {
        self.environment
            .iter()
            .chain(&self.transcripts)
            .filter_map(readonly_shell_file)
            .collect()
    }
}

/// Shell state follows the session across runtime configuration reloads.
pub struct ImportedShellSessions {
    sessions: Mutex<HashMap<i64, Arc<SessionShell>>>,
    watches: Arc<FileWatches>,
    plugins: Mutex<Weak<PluginHost>>,
    shutdown: CancellationToken,
    reader: KernelReader,
}

#[derive(Default)]
pub(super) struct SessionShell {
    state: Mutex<ShellState>,
    updates: tokio::sync::Mutex<()>,
    end: CancellationToken,
    transcripts: SessionTranscripts,
}

#[derive(Default)]
struct ShellState {
    cwd: Option<PathBuf>,
    environment_enabled: bool,
    environment: Option<ShellEnvironment>,
}

struct ShellEnvironment {
    file: ImportedHookEnvironment,
    reader: Arc<Mutex<std::fs::File>>,
    owners: BTreeMap<PluginKey, PathBuf>,
}

impl ShellEnvironment {
    fn new() -> io::Result<Self> {
        let directory = private_directory()?;
        let path = directory.path().join("environment.sh");
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let reader = Arc::new(Mutex::new(options.open(&path)?));
        Ok(Self {
            file: ImportedHookEnvironment::new(path, Arc::new(directory)),
            reader,
            owners: BTreeMap::new(),
        })
    }

    fn is_active(&self, host: &PluginHost) -> bool {
        self.owners.iter().all(|(key, root)| {
            host.plugins().iter().any(|plugin| {
                plugin.key() == *key
                    && matches!(
                        &plugin.configured_plugin().package,
                        PluginPackage::AgentPlugin {
                            enable_hooks: true,
                            ..
                        }
                    )
                    && plugin
                        .agent_plugin()
                        .is_some_and(|package| package.root == *root)
            })
        })
    }
}

impl ImportedShellSessions {
    pub fn new(watches: Arc<FileWatches>, plugins: &Arc<PluginHost>, reader: KernelReader) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            watches,
            plugins: Mutex::new(Arc::downgrade(plugins)),
            shutdown: CancellationToken::new(),
            reader,
        }
    }

    pub fn set_plugins(&self, plugins: &Arc<PluginHost>) {
        *self
            .plugins
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Arc::downgrade(plugins);
        for shell in self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
        {
            let mut state = shell
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state
                .environment
                .as_ref()
                .is_some_and(|environment| !environment.is_active(plugins))
            {
                state.environment = None;
            }
        }
    }

    fn plugins(&self) -> Option<Arc<PluginHost>> {
        self.plugins
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .upgrade()
    }

    fn get(&self, session: i64) -> Option<Arc<SessionShell>> {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&session)
            .cloned()
    }

    fn open(&self, session: i64) -> Option<Arc<SessionShell>> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.shutdown.is_cancelled() {
            return None;
        }
        Some(sessions.entry(session).or_default().clone())
    }

    pub fn end_session(&self, session: i64) {
        if let Some(shell) = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&session)
        {
            shell.end.cancel();
        }
    }

    pub fn close(&self) {
        self.shutdown.cancel();
        for shell in self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .drain()
            .map(|(_, shell)| shell)
        {
            shell.end.cancel();
        }
    }

    pub fn has_cwd(&self, session: i64) -> bool {
        self.get(session).is_some_and(|shell| {
            shell
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .cwd
                .is_some()
        })
    }

    pub fn session_lifetime(&self, session: i64) -> Option<CancellationToken> {
        self.open(session).map(|shell| shell.end.child_token())
    }

    pub async fn transcript_for_hook(
        &self,
        session: SessionId,
        workspace: &Path,
        format: HookTranscriptFormat,
        plugin: PluginKey,
        root: PathBuf,
    ) -> Result<Option<ImportedHookFile>, TranscriptError> {
        let Some(shell) = self.open(session.get()) else {
            return Ok(None);
        };
        tokio::select! {
            biased;
            _ = shell.end.cancelled() => Ok(None),
            result = shell.transcripts.for_hook(&self.reader, session, workspace, format, (plugin, root)) => result.map(Some),
        }
    }

    pub async fn environment_for_hook(
        self: &Arc<Self>,
        session: i64,
        plugin: PluginKey,
        root: PathBuf,
    ) -> io::Result<Option<ImportedHookEnvironment>> {
        let Some(shell) = self.open(session) else {
            return Ok(None);
        };
        ENVIRONMENT_WORKERS
            .run(move || {
                let mut state = shell
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if shell.end.is_cancelled() {
                    return Ok(None);
                }
                if state.environment.is_none() {
                    state.environment = Some(ShellEnvironment::new()?);
                }
                state.environment_enabled = true;
                let environment = state
                    .environment
                    .as_mut()
                    .expect("shell environment was created");
                environment.owners.insert(plugin, root);
                Ok(Some(environment.file.clone()))
            })
            .await
            .map_err(io::Error::other)?
    }
}

impl ToolExecutor {
    pub fn with_imported_shells(mut self, shells: Arc<ImportedShellSessions>) -> Self {
        self.imported_shells = Some(shells);
        self
    }

    pub(super) async fn imported_shell_files(
        &self,
        source_environment: bool,
    ) -> ImportedShellFiles {
        let environment = if source_environment && self.sandbox_enabled() {
            self.shell_environment(|environment| environment.file.clone())
                .filter(|file| file.path().to_str().is_some())
        } else {
            None
        };
        ImportedShellFiles {
            environment,
            transcripts: self.imported_shell_transcripts().await,
        }
    }

    async fn imported_shell_transcripts(&self) -> Vec<ImportedHookFile> {
        if self.is_global() || !self.sandbox_enabled() {
            return Vec::new();
        }
        let Some(sessions) = &self.imported_shells else {
            return Vec::new();
        };
        let Some(session) = self.session_id.and_then(|id| SessionId::try_from(id).ok()) else {
            return Vec::new();
        };
        let Some(shell) = sessions.get(session.get()) else {
            return Vec::new();
        };
        let formats = shell
            .transcripts
            .active_formats(|key, root| {
                self.plugin_is_enabled(key)
                    && self.plugin_manager().plugins().iter().any(|plugin| {
                        plugin.key() == *key
                            && matches!(
                                plugin.configured_plugin().package,
                                PluginPackage::AgentPlugin {
                                    enable_hooks: true,
                                    ..
                                }
                            )
                            && plugin
                                .agent_plugin()
                                .is_some_and(|package| package.root == root)
                    })
            })
            .await;
        let mut files = Vec::with_capacity(formats.len());
        for format in formats {
            let cancellation = self.cancellation_token().cloned().unwrap_or_default();
            let result = tokio::select! {
                biased;
                _ = shell.end.cancelled() => break,
                _ = cancellation.cancelled() => break,
                result = shell.transcripts.refresh(&sessions.reader, session, self.workspace_root(), format, None) => result,
            };
            match result {
                Ok(file) => files.push(file),
                Err(error) => {
                    tracing::warn!(%error, "could not refresh a hook transcript for the session shell")
                }
            }
        }
        files
    }

    fn imported_directory_session(&self) -> Option<i64> {
        if self.session_kind != SessionKind::Project
            || self.session_relation_kind.is_subagent()
            || self.is_global()
        {
            return None;
        }
        self.session_id.filter(|id| *id > 0)
    }

    pub(crate) fn tracks_imported_shell_cwd(&self) -> bool {
        let (Some(shells), Some(session)) =
            (&self.imported_shells, self.imported_directory_session())
        else {
            return false;
        };
        [
            HookSubscription::SESSION_CWD_CHANGED,
            HookSubscription::SESSION_FILE_CHANGED,
            HookSubscription::IMPORTED_DIRECTORY_ADDED,
        ]
        .into_iter()
        .any(|hook| self.plugin_manager().has_subscription(hook))
            || shells.get(session).is_some_and(|shell| {
                shell
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .environment_enabled
            })
    }

    pub fn imported_shell_cwd(&self) -> Option<PathBuf> {
        if !self.tracks_imported_shell_cwd() {
            return None;
        }
        let shell = self
            .imported_shells
            .as_ref()?
            .get(self.imported_directory_session()?)?;
        let path = shell
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .cwd
            .clone()?;
        let resolved = path.canonicalize().ok()?;
        (resolved.is_dir() && self.imported_shell_path_allowed(&resolved)).then_some(resolved)
    }

    fn shell_environment<T>(&self, select: impl FnOnce(&ShellEnvironment) -> T) -> Option<T> {
        let shells = self.imported_shells.as_ref()?;
        let plugins = shells.plugins()?;
        let shell = shells.get(self.session_id?)?;
        let state = shell
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let environment = state.environment.as_ref()?;
        (environment.is_active(&plugins)
            && environment
                .owners
                .keys()
                .all(|plugin| self.plugin_is_enabled(plugin)))
        .then(|| select(environment))
    }

    pub(super) async fn imported_powershell_environment(&self) -> HashMap<String, String> {
        if self.sandbox_enabled() {
            return HashMap::new();
        }
        let Some(reader) = self.shell_environment(|environment| environment.reader.clone()) else {
            return HashMap::new();
        };
        const MAX_ENVIRONMENT_BYTES: u64 = 64 * 1024;
        let assignments = ENVIRONMENT_WORKERS
            .run(move || {
                let mut reader = reader.lock().unwrap_or_else(|error| error.into_inner());
                let mut text = String::new();
                reader.seek(SeekFrom::Start(0)).ok()?;
                reader
                    .by_ref()
                    .take(MAX_ENVIRONMENT_BYTES + 1)
                    .read_to_string(&mut text)
                    .ok()?;
                if text.len() as u64 > MAX_ENVIRONMENT_BYTES {
                    return None;
                }
                assignments::parse(&text)
            })
            .await
            .ok()
            .flatten();
        match assignments {
            Some(mut assignments) => {
                if cfg!(windows) {
                    assignments.retain(|_, value| !value.contains(['/', '\\']));
                }
                assignments
            }
            None => {
                tracing::debug!("session shell environment is not all plain assignments");
                HashMap::new()
            }
        }
    }

    fn imported_shell_path_allowed(&self, path: &Path) -> bool {
        matches!(
            self.sandbox_permission_snapshot()
                .authorize_path_access(AccessKind::Read, path),
            PermissionDecision::Allow
        )
    }

    pub(super) fn track_imported_shell(&self, call: Option<i64>) -> Option<Arc<SessionShell>> {
        if !call.is_some_and(|call| call >= 0) || !self.tracks_imported_shell_cwd() {
            return None;
        }
        self.imported_shells
            .as_ref()?
            .open(self.imported_directory_session()?)
    }

    pub(super) async fn record_imported_shell_cwd(
        &self,
        shell: &SessionShell,
        launch_dir: &Path,
        path: PathBuf,
    ) {
        let (Some(sessions), Some(session)) =
            (&self.imported_shells, self.imported_directory_session())
        else {
            return;
        };
        let Ok(path) = tokio::fs::canonicalize(path).await else {
            return;
        };
        if path == launch_dir
            || !path.is_dir()
            || !self.imported_shell_path_allowed(&path)
            || self.ensure_not_cancelled().is_err()
        {
            return;
        }
        let _update = shell.updates.lock().await;
        if shell.end.is_cancelled() || self.ensure_not_cancelled().is_err() {
            return;
        }
        let old = {
            let mut state = shell
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let previous = state.cwd.as_deref().unwrap_or(self.workspace_root());
            if previous == path {
                return;
            }
            state.environment = None;
            state
                .cwd
                .replace(path.clone())
                .unwrap_or_else(|| self.workspace_root().to_path_buf())
        };
        sessions.watches.refresh_session(session);
        let Some(plugins) = sessions.plugins() else {
            return;
        };
        tokio::select! {
            biased;
            _ = shell.end.cancelled() => {}
            _ = plugins.dispatch_imported_session_cwd_changed(SessionCwdChangedInput {
                session_id: session,
                old_cwd: old,
                new_cwd: path,
            }) => {}
        }
    }
}

fn readonly_shell_file(file: &ImportedHookFile) -> Option<PrivateShellFile> {
    Some(PrivateShellFile {
        directory: file.path().parent()?.to_path_buf(),
        path: file.path().to_path_buf(),
        writable: false,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use agena_tool::shell::{ShellDialect, ShellLaunchSpec};

    #[test]
    fn directory_capture_preserves_shell_result_and_ignores_subshell_cd() {
        let workspace = tempfile::tempdir().unwrap();
        let nested = workspace.path().join("dir with 'quotes'");
        std::fs::create_dir(&nested).unwrap();
        let shell = ShellLaunchSpec {
            dialect: ShellDialect::Sh,
            program: "/bin/sh".into(),
            ..Default::default()
        };
        let capture = DirectoryCapture::new().unwrap();
        let target = quote(nested.to_str().unwrap());
        let argv = capture
            .argv(
                &shell,
                &format!("cd {target}; (cd /); printf result; exit 7"),
            )
            .unwrap();
        let output = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(workspace.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"result");
        assert!(output.stderr.is_empty());
        assert_eq!(capture.into_cwd(), Some(nested.canonicalize().unwrap()));

        let capture = DirectoryCapture::new().unwrap();
        let argv = capture
            .argv(&shell, &format!("trap 'printf cleanup' 0; cd {target}"))
            .unwrap();
        let output = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(workspace.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output.stderr);
        assert_eq!(output.stdout, b"cleanup");
        assert_eq!(capture.into_cwd(), Some(nested.canonicalize().unwrap()));
    }

    #[test]
    fn bash_sources_the_shared_hook_environment_as_shell_code() {
        let environment = ShellEnvironment::new().unwrap();
        std::fs::write(
            environment.file.path(),
            "export AGENA_HOOK_TEST=\"$AGENA_HOOK_TEST:new\"\n",
        )
        .unwrap();
        let script =
            source_environment(&environment.file, "printf '%s' \"$AGENA_HOOK_TEST\"").unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &script])
            .env("AGENA_HOOK_TEST", "previous")
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output.stderr);
        assert_eq!(output.stdout, b"previous:new");
    }
}
