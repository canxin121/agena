//! Native MXC execution shared by shell, terminal and tool subprocesses.
//!
//! Permission decisions belong to the caller. This module transports an already
//! approved policy to a short-lived launcher, so existing Tokio pipes, PTYs and
//! process-tree ownership continue to work. Launch failures never run the command
//! outside the sandbox unless isolation was explicitly disabled by the host.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use agena_domain::{
    NetworkAction as ConfigNetworkAction, NetworkPermissionConfig,
    NetworkProtocol as ConfigNetworkProtocol, NetworkRuleConfig,
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mxc_sdk::v1::{
    ContainerRequest, FilesystemPolicy, NetworkAction, NetworkEgressPolicy, NetworkIngressPolicy,
    NetworkPeerPolicy, NetworkPolicy, NetworkPortPolicy, NetworkRulePolicy, SpawnExtendedOptions,
    SpawnStdio, UiPolicy, WaitResult,
};
use serde::{Deserialize, Serialize};

mod configured;
mod host_access;
mod path_aliases;

const LAUNCH_ARGUMENT: &str = "--agena-sandbox-launch";
const PAYLOAD_ARGUMENT: &str = "--agena-sandbox-payload";
const PAYLOAD_ENV_PREFIX: &str = "AGENA_SANDBOX_PAYLOAD_ENV_";
const LAUNCH_FAILURE: i32 = 125;
static LAUNCHER_PATH: OnceLock<PathBuf> = OnceLock::new();
static SANDBOX_ENABLED: OnceLock<bool> = OnceLock::new();

tokio::task_local! {
    static INVOCATION_SANDBOX_ENABLED: bool;
}

/// Trusted process setting, captured once rather than read from tool environments.
/// `AGENA_SANDBOX=disabled` disables MXC for this Agena process. The historical
/// shell setting remains an alias when the process-wide setting is absent.
pub fn enabled() -> bool {
    *SANDBOX_ENABLED.get_or_init(|| enabled_from_lookup(|key| std::env::var(key).ok()))
        && INVOCATION_SANDBOX_ENABLED
            .try_with(|enabled| *enabled)
            .unwrap_or(true)
}

/// Apply a trusted invocation's mode to async helpers without changing other
/// requests. Blocking work and cached processes must carry `policy.disabled`
/// explicitly, since Tokio task-local values do not cross task boundaries.
pub async fn scope_enabled<F: std::future::Future>(enabled: bool, future: F) -> F::Output {
    INVOCATION_SANDBOX_ENABLED.scope(enabled, future).await
}

fn enabled_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> bool {
    lookup("AGENA_SANDBOX")
        .or_else(|| lookup("AGENA_SHELL_SANDBOX"))
        .is_none_or(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "disabled" | "false" | "0" | "off"
            )
        })
}

/// Concrete permissions for one process tree. Values are supplied by trusted
/// runtime code after the normal permission/approval flow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxPolicy {
    /// Host-selected execution mode, never populated from model tool input.
    /// Included in policy equality so isolated and direct processes cannot share
    /// a cached LSP, MCP connection or plugin process.
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub readonly_paths: Vec<PathBuf>,
    #[serde(default)]
    pub readwrite_paths: Vec<PathBuf>,
    #[serde(default)]
    pub denied_paths: Vec<PathBuf>,
    /// A complete, host-compiled configuration. Broad grants are partitioned
    /// around private host resources before they reach the native backend.
    #[serde(default)]
    pub configured_filesystem: bool,
    /// Class defaults that remove an inherited grant while allowing separately
    /// granted descendants. These are not explicit MXC denied paths.
    #[serde(default)]
    pub default_denied_paths: Vec<PathBuf>,
    /// Host storage boundaries excluded from implicit runtime dependencies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub private_storage_roots: Vec<PathBuf>,
    /// Additional installation files required by a configured tool or language server.
    #[serde(default)]
    pub runtime_paths: Vec<PathBuf>,
    /// Existing temporary directories owned by this trusted caller. This only
    /// permits grants into reserved scratch namespaces; it grants no access by
    /// itself and must never be filled from a tool's resource declaration.
    #[serde(default)]
    pub private_paths: Vec<PathBuf>,
    /// Caller-owned private directory to use as the workload's HOME.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_home: Option<PathBuf>,
    #[serde(default)]
    pub network: SandboxNetwork,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Language-server build products stay in its private scratch directory.
    #[serde(default)]
    pub private_build_cache: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SandboxNetwork {
    #[default]
    Disabled,
    /// Native MXC IP, protocol and port policy. No userspace proxy is involved.
    Native { policy: NetworkPermissionConfig },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LauncherRequest {
    argv: Vec<String>,
    cwd: PathBuf,
    temporary_root: PathBuf,
    #[serde(skip)]
    environment: HashMap<String, String>,
    environment_keys: Vec<String>,
    policy: SandboxPolicy,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkloadPayload {
    argv: Vec<String>,
    cwd: PathBuf,
    environment: HashMap<String, String>,
}

/// Set a trusted launcher executable for embedders and integration-test helpers.
/// The executable must call try_run_launcher before its ordinary argument parser.
pub fn configure_launcher(path: impl Into<PathBuf>) -> io::Result<()> {
    let path = path.into();
    if !path.is_absolute() || !path.is_file() {
        return Err(invalid(
            "sandbox launcher must be an existing absolute executable path",
        ));
    }
    let existing = LAUNCHER_PATH.get_or_init(|| path.clone());
    if existing == &path {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "sandbox launcher is already configured",
        ))
    }
}

/// Resolve the trusted launcher used for sandbox and application worker modes.
pub fn launcher_executable() -> io::Result<PathBuf> {
    LAUNCHER_PATH
        .get()
        .cloned()
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)
}

/// Retain terminal, locale and installed-runtime discovery settings, without
/// copying ambient API credentials, agents, preload hooks or proxy credentials.
/// Explicit overrides added by trusted callers of wrap_tokio_command are retained.
pub fn sanitize_environment(environment: &HashMap<String, String>) -> HashMap<String, String> {
    environment
        .iter()
        .filter(|(name, _)| safe_environment_name(name))
        .map(|(name, value)| (environment_key(name), value.clone()))
        .collect()
}

fn environment_key(name: &str) -> String {
    if cfg!(windows) {
        name.to_ascii_uppercase()
    } else {
        name.to_owned()
    }
}

fn safe_environment_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("LC_")
        || matches!(
            upper.as_str(),
            "PATH"
                | "PATHEXT"
                | "SYSTEMROOT"
                | "WINDIR"
                | "COMSPEC"
                | "LANG"
                | "LANGUAGE"
                | "TERM"
                | "COLORTERM"
                | "NO_COLOR"
                | "FORCE_COLOR"
                | "NUMBER_OF_PROCESSORS"
                | "PROCESSOR_ARCHITECTURE"
                | "RUSTUP_HOME"
                | "RUSTUP_TOOLCHAIN"
                | "JAVA_HOME"
                | "GOROOT"
                | "DOTNET_ROOT"
                | "VIRTUAL_ENV"
                | "CONDA_PREFIX"
        )
}

/// When isolation is enabled, replace argv with the sandbox launcher argv and
/// reduce the environment. Direct execution keeps argv and environment intact.
pub fn wrap_argv(
    argv: &[String],
    cwd: &Path,
    environment: &mut HashMap<String, String>,
    policy: &SandboxPolicy,
) -> io::Result<Vec<String>> {
    if !enabled() || policy.disabled {
        if argv.is_empty() {
            return Err(invalid("process argv must not be empty"));
        }
        return Ok(argv.to_vec());
    }
    let mut safe = sanitize_environment(environment);
    // Rustup's shims find installed toolchains through RUSTUP_HOME even though
    // the workload's HOME and CARGO_HOME are private.
    capture_rustup_home(environment, &mut safe);
    let (wrapped, launcher_environment) = launcher_argv(argv, cwd, &safe, policy)?;
    *environment = launcher_environment;
    Ok(wrapped)
}

/// Construct a Tokio command in the selected mode. Set stdio on the result.
pub fn command(
    program: &str,
    args: &[String],
    cwd: &Path,
    environment: &HashMap<String, String>,
    policy: &SandboxPolicy,
) -> io::Result<tokio::process::Command> {
    if !enabled() || policy.disabled {
        let mut command = tokio::process::Command::new(program);
        command.args(args).current_dir(cwd).envs(environment);
        return Ok(command);
    }
    let mut argv = vec![program.to_owned()];
    argv.extend_from_slice(args);
    let mut environment = environment.clone();
    let argv = wrap_argv(&argv, cwd, &mut environment, policy)?;
    let mut command = tokio::process::Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .env_clear()
        .envs(environment);
    Ok(command)
}

/// Wrap an existing command BEFORE configuring stdin/stdout/stderr or platform
/// creation flags. Rust exposes no way to transfer those opaque settings when
/// replacing the executable. Prefer command or wrap_argv for new call sites.
pub fn wrap_tokio_command(
    command: &mut tokio::process::Command,
    policy: &SandboxPolicy,
) -> io::Result<()> {
    if !enabled() || policy.disabled {
        return Ok(());
    }
    let captured = capture_command(command.as_std(), policy)?;
    let (argv, launcher_environment) = launcher_argv(
        &captured.argv,
        &captured.cwd,
        &captured.environment,
        &captured.policy,
    )?;
    let mut wrapped = tokio::process::Command::new(&argv[0]);
    wrapped
        .args(&argv[1..])
        .current_dir(&captured.cwd)
        .env_clear()
        .envs(launcher_environment);
    *command = wrapped;
    Ok(())
}

/// Synchronous counterpart with the same before-stdio requirement.
pub fn wrap_std_command(
    command: &mut std::process::Command,
    policy: &SandboxPolicy,
) -> io::Result<()> {
    if !enabled() || policy.disabled {
        return Ok(());
    }
    let captured = capture_command(command, policy)?;
    let (argv, launcher_environment) = launcher_argv(
        &captured.argv,
        &captured.cwd,
        &captured.environment,
        &captured.policy,
    )?;
    let mut wrapped = std::process::Command::new(&argv[0]);
    wrapped
        .args(&argv[1..])
        .current_dir(&captured.cwd)
        .env_clear()
        .envs(launcher_environment);
    *command = wrapped;
    Ok(())
}

fn capture_command(
    command: &std::process::Command,
    policy: &SandboxPolicy,
) -> io::Result<LauncherRequest> {
    let mut argv = vec![utf8(command.get_program(), "program")?.to_owned()];
    for argument in command.get_args() {
        argv.push(utf8(argument, "argument")?.to_owned());
    }
    let cwd = command
        .get_current_dir()
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)?;
    let ambient = ambient_environment();
    let mut environment = sanitize_environment(&ambient);
    capture_rustup_home(&ambient, &mut environment);
    for (name, value) in command.get_envs() {
        let name = environment_key(utf8(name, "environment name")?);
        match value {
            Some(value) => {
                reject_loader_environment(&name)?;
                environment.insert(name, utf8(value, "environment value")?.to_owned());
            }
            None => {
                environment.remove(&name);
            }
        }
    }
    Ok(LauncherRequest {
        argv,
        cwd,
        temporary_root: capture_temporary_root()?,
        environment_keys: environment.keys().cloned().collect(),
        environment,
        policy: policy.clone(),
    })
}

fn launcher_argv(
    argv: &[String],
    cwd: &Path,
    environment: &HashMap<String, String>,
    policy: &SandboxPolicy,
) -> io::Result<(Vec<String>, HashMap<String, String>)> {
    let temporary_root = capture_temporary_root()?;
    let executable = launcher_executable()?;
    let mut launcher_environment = trusted_launcher_environment();
    let policy = configured::prepare(
        policy,
        &temporary_root,
        &executable,
        cwd,
        &launcher_environment,
    )?;
    validate_policy_for_root(&policy, &temporary_root)?;
    if argv.is_empty() || argv[0].is_empty() || argv.iter().any(|s| s.contains('\0')) {
        return Err(invalid(
            "sandbox requires a nonempty program and NUL-free arguments",
        ));
    }
    if !cwd.is_absolute() {
        return Err(invalid("sandbox working directory must be absolute"));
    }
    let request = LauncherRequest {
        argv: argv.to_vec(),
        cwd: cwd.to_path_buf(),
        temporary_root,
        environment_keys: environment.keys().cloned().collect(),
        environment: environment.clone(),
        policy: policy.clone(),
    };
    let bytes = serde_json::to_vec(&request).map_err(invalid)?;
    let encoded = URL_SAFE_NO_PAD.encode(bytes);
    // CreateProcess has a 32,767 UTF-16 command-line limit. Fail before spawn
    // instead of truncating a policy or silently dropping arguments.
    #[cfg(windows)]
    if encoded.len() > 28_000 {
        return Err(invalid(
            "sandbox launch request exceeds the Windows command-line limit",
        ));
    }
    #[cfg(not(windows))]
    if encoded.len() > 120_000 {
        return Err(invalid(
            "sandbox launch request is too large for one command-line argument",
        ));
    }
    reject_writable_launcher_paths(&policy, &executable, cwd, &launcher_environment)?;
    for (index, name) in request.environment_keys.iter().enumerate() {
        reject_loader_environment(name)?;
        launcher_environment.insert(
            format!("{PAYLOAD_ENV_PREFIX}{index}"),
            environment[name].clone(),
        );
    }
    Ok((
        vec![
            utf8(executable.as_os_str(), "launcher path")?.to_owned(),
            LAUNCH_ARGUMENT.to_owned(),
            encoded,
        ],
        launcher_environment,
    ))
}

// A readonly bind of one executable does not protect its host parent directory
// from replacement. Keep every writable task resource outside the native
// launcher's executable and backend lookup paths, including symlink spellings.
fn reject_writable_launcher_paths(
    policy: &SandboxPolicy,
    launcher: &Path,
    cwd: &Path,
    native_environment: &HashMap<String, String>,
) -> io::Result<()> {
    if policy.readwrite_paths.is_empty() {
        return Ok(());
    }
    let protected = protected_launcher_paths(launcher, cwd, native_environment)?;
    let writes: Vec<_> = policy
        .readwrite_paths
        .iter()
        .map(|grant| Ok((grant, policy_path_identity(grant)?)))
        .collect::<io::Result<_>>()?;
    for (protected_path, is_search_directory) in protected {
        let identities = protected_path_identities(&protected_path)?;
        for (grant, canonical_grant) in &writes {
            let conflicts = identities.iter().any(|protected| {
                [grant.as_path(), canonical_grant.as_path()]
                    .iter()
                    .any(|write| {
                        identity_within(protected, write)
                            || (is_search_directory && identity_within(write, protected))
                    })
            });
            if conflicts {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "sandbox writable resource {} overlaps trusted sandbox launcher/backend path {}; install Agena and backend tools outside writable task trees",
                        grant.display(),
                        protected_path.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn protected_path_identities(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut identities = vec![path.to_path_buf(), policy_path_identity(path)?];
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        // Retain the replaceable directory entry as well as a symlink target.
        identities.push(policy_path_identity(parent)?.join(name));
    }
    Ok(identities)
}

fn protected_launcher_paths(
    launcher: &Path,
    cwd: &Path,
    native_environment: &HashMap<String, String>,
) -> io::Result<Vec<(PathBuf, bool)>> {
    let mut protected = Vec::new();
    for executable in [launcher.to_path_buf(), std::env::current_exe()?] {
        // Keep installation trees immutable, including a symlink entry point's
        // real installation directory. Protecting only one binary would split
        // large build/install directories into a mount for every sibling file.
        for identity in protected_path_identities(&executable)? {
            if let Some(directory) = identity.parent() {
                protected.push((directory.to_path_buf(), true));
            }
        }
        protected.push((executable, false));
    }
    let path = native_environment.get("PATH");
    if let Some(path) = path {
        for directory in std::env::split_paths(path) {
            let directory = if directory.is_absolute() {
                directory
            } else {
                cwd.join(directory)
            };
            // Also prevent planting a backend executable earlier in PATH.
            protected.push((directory, true));
        }
    }
    #[cfg(target_os = "linux")]
    for name in [
        "bwrap",
        "unshare",
        "sh",
        "slirp4netns",
        "nsenter",
        "ip",
        "iptables",
        "ip6tables",
        "iptables-restore",
        "ip6tables-restore",
        "sleep",
    ] {
        if let Ok(executable) = which::which_in(name, path, cwd) {
            protected.push((executable, false));
        }
    }
    #[cfg(unix)]
    protected.push((PathBuf::from("/bin/sh"), false));
    #[cfg(target_os = "macos")]
    protected.push((PathBuf::from("/usr/bin/sandbox-exec"), false));
    #[cfg(windows)]
    {
        if let Some(shell) = native_environment.get("COMSPEC") {
            protected.push((PathBuf::from(shell), false));
        }
        if let Some(system) = native_environment.get("SYSTEMROOT") {
            protected.push((PathBuf::from(system), true));
        }
    }

    let mut libraries = Vec::new();
    for (path, is_directory) in &protected {
        let directory = if *is_directory {
            Some(path.as_path())
        } else {
            path.parent()
        };
        if let Some(directory) = directory {
            add_installation_libraries(directory, &mut libraries);
        }
    }
    #[cfg(target_os = "macos")]
    libraries.push(PathBuf::from("/System/Library"));
    protected.extend(libraries.into_iter().map(|path| (path, true)));
    Ok(protected)
}

fn ambient_environment() -> HashMap<String, String> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

fn trusted_launcher_environment() -> HashMap<String, String> {
    let ambient = ambient_environment();
    let mut environment = sanitize_environment(&ambient);
    // These paths belong only to MXC's trusted setup process. The workload
    // receives independent private directories in its payload environment.
    for (name, value) in ambient {
        if matches!(
            name.to_ascii_uppercase().as_str(),
            "HOME"
                | "USERPROFILE"
                | "HOMEDRIVE"
                | "HOMEPATH"
                | "APPDATA"
                | "LOCALAPPDATA"
                | "TMPDIR"
                | "TMP"
                | "TEMP"
        ) {
            environment.insert(environment_key(&name), value);
        }
    }
    environment
}

fn reject_loader_environment(name: &str) -> io::Result<()> {
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("LD_")
        || upper.starts_with("DYLD_")
        || matches!(
            upper.as_str(),
            "GLIBC_TUNABLES" | "GCONV_PATH" | "LIBPATH" | "SHLIB_PATH"
        )
    {
        return Err(invalid(format!(
            "sandbox environment variable {name} controls dynamic loading and cannot be accepted; use a trusted runtime installation instead"
        )));
    }
    Ok(())
}

/// Handle the internal launcher before parsing application arguments or opening
/// application state. None means ordinary application startup should continue.
pub fn try_run_launcher() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    let argument = args.next();
    let is_payload = argument.as_deref() == Some(OsStr::new(PAYLOAD_ARGUMENT));
    if !is_payload && argument.as_deref() != Some(OsStr::new(LAUNCH_ARGUMENT)) {
        return None;
    }
    let result = (|| {
        let encoded = args
            .next()
            .ok_or_else(|| invalid("missing sandbox launch request"))?;
        if args.next().is_some() {
            return Err(invalid("unexpected sandbox launcher arguments"));
        }
        if is_payload {
            return run_workload_payload(Path::new(&encoded));
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(utf8(&encoded, "sandbox launch request")?)
            .map_err(invalid)?;
        let request: LauncherRequest = serde_json::from_slice(&decoded).map_err(invalid)?;
        run_launcher(request)
    })();
    Some(match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Agena sandbox: {error}");
            LAUNCH_FAILURE
        }
    })
}

// This entry point runs inside MXC. Its file is created privately by the parent
// launcher, so trusted environment values never become
// bwrap --setenv command-line arguments visible through /proc/*/cmdline.
fn run_workload_payload(path: &Path) -> io::Result<i32> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(invalid("sandbox workload payload is too large"));
    }
    let payload: WorkloadPayload = serde_json::from_slice(&bytes).map_err(invalid)?;
    let (program, args) = payload
        .argv
        .split_first()
        .ok_or_else(|| invalid("empty sandbox workload argv"))?;
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .current_dir(&payload.cwd)
        .env_clear()
        .envs(payload.environment);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec())
    }
    #[cfg(not(unix))]
    {
        Ok(command.status()?.code().unwrap_or(1))
    }
}

fn run_launcher(mut payload: LauncherRequest) -> io::Result<i32> {
    validate_policy_for_root(&payload.policy, &payload.temporary_root)?;
    // Values use inert transport names. Workload PATH, SystemRoot, COMSPEC,
    // and other settings cannot alter this launcher's native environment.
    payload.environment = payload
        .environment_keys
        .iter()
        .enumerate()
        .map(|(index, name)| {
            reject_loader_environment(name)?;
            let value = std::env::var(format!("{PAYLOAD_ENV_PREFIX}{index}"))
                .map_err(|_| invalid("sandbox payload environment transport is incomplete"))?;
            Ok((name.clone(), value))
        })
        .collect::<io::Result<HashMap<_, _>>>()?;
    let scratch = tempfile::Builder::new()
        .prefix("agena-sandbox-")
        .tempdir_in(&payload.temporary_root)?;
    let request = mxc_request(&payload, scratch.path())?;
    let mut options = SpawnExtendedOptions::default();
    options.stdio = SpawnStdio::Inherit;
    install_termination_handlers()?;
    #[cfg(unix)]
    let parent = unsafe { libc::getppid() };
    let started = Instant::now();
    let mut process =
        mxc_sdk::v1::spawn_with_extended_options(request, options).map_err(|error| {
            io::Error::other(format!(
                "MXC could not start the command: {error}. {}",
                prerequisite_hint()
            ))
        })?;
    let result = loop {
        match process.try_wait() {
            Ok(Some(_)) => break process.wait().map(wait_exit_code),
            Ok(None) => {}
            Err(error) if error.kind() == io::ErrorKind::TimedOut => break Ok(124),
            Err(error) => break Err(error),
        }
        if let Some(milliseconds) = payload.policy.timeout_ms
            && started.elapsed() >= Duration::from_millis(milliseconds)
        {
            process.kill_for_timeout()?;
            let _ = process.wait()?;
            break Ok(124);
        }
        #[cfg(unix)]
        {
            let signal = TERMINATION_SIGNAL.load(std::sync::atomic::Ordering::Relaxed);
            if signal != 0 || unsafe { libc::getppid() } != parent {
                process.kill()?;
                let _ = process.wait()?;
                break Ok(if signal != 0 { 128 + signal } else { 143 });
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // MXC tears down remaining descendants before private files disappear,
    // including descendants retaining stdio.
    for warning in process.warnings() {
        eprintln!("Agena sandbox: {warning}");
    }
    drop(process);
    drop(scratch);
    result
}

fn wait_exit_code(outcome: WaitResult) -> i32 {
    match outcome {
        WaitResult::Exited(code) if code >= 0 => code,
        WaitResult::Exited(_) => 1,
        WaitResult::TimedOut => 124,
    }
}

fn mxc_request(payload: &LauncherRequest, scratch: &Path) -> io::Result<ContainerRequest> {
    let mut dependencies = payload.policy.runtime_paths.clone();
    dependencies.extend(runtime_paths(
        &payload.argv,
        &payload.cwd,
        &payload.environment,
    )?);
    let bootstrap = std::env::current_exe()?;
    add_executable_paths(&bootstrap, &mut dependencies)?;
    reject_private_storage_dependencies(&payload.policy, &dependencies)?;
    let mut readonly_paths = payload.policy.readonly_paths.clone();
    readonly_paths.extend(dependencies);
    reject_shared_temporary_grants(
        &readonly_paths,
        &payload.temporary_root,
        &payload.policy.private_paths,
    )?;
    reject_denied_runtime(&payload.policy, &readonly_paths)?;
    reject_denied_runtime(&payload.policy, &[scratch.to_path_buf()])?;
    let mut environment = payload.environment.clone();
    private_environment(&mut environment, scratch)?;
    if let Some(home) = &payload.policy.workload_home {
        let home = utf8(home.as_os_str(), "workload home")?;
        environment.insert("HOME".to_owned(), home.to_owned());
        environment.insert("USERPROFILE".to_owned(), home.to_owned());
    }
    if payload.policy.private_build_cache {
        environment.retain(|name, _| !name.eq_ignore_ascii_case("CARGO_TARGET_DIR"));
        let target = scratch.join("cache").join("cargo-target");
        std::fs::create_dir_all(&target)?;
        environment.insert(
            "CARGO_TARGET_DIR".to_owned(),
            utf8(target.as_os_str(), "private build directory")?.to_owned(),
        );
    }
    // MXC's initial environment is intentionally free of private overrides.
    // The bootstrap restores those from a mode-0600 file after isolation.
    let mut bootstrap_environment = sanitize_environment(&environment);
    private_environment(&mut bootstrap_environment, scratch)?;
    let workload = WorkloadPayload {
        argv: payload.argv.clone(),
        cwd: payload.cwd.clone(),
        environment,
    };
    let payload_path = scratch.join("workload.json");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    serde_json::to_writer(options.open(&payload_path)?, &workload).map_err(invalid)?;
    let bootstrap_argv = vec![
        utf8(bootstrap.as_os_str(), "bootstrap path")?.to_owned(),
        PAYLOAD_ARGUMENT.to_owned(),
        utf8(payload_path.as_os_str(), "payload path")?.to_owned(),
    ];
    let mut readwrite_paths = payload.policy.readwrite_paths.clone();
    readwrite_paths.push(scratch.to_path_buf());
    let mut request = ContainerRequest::new(command_line(&bootstrap_argv)?);
    request.filesystem = Some(FilesystemPolicy {
        readonly_paths: path_strings(readonly_paths)?,
        readwrite_paths: path_strings(readwrite_paths)?,
        denied_paths: path_strings(payload.policy.denied_paths.clone())?,
        clear_policy_on_exit: Some(true),
    });
    request.network = Some(network_policy(&payload.policy.network)?);
    request.ui = Some(UiPolicy::default());
    request.working_directory =
        Some(utf8(payload.cwd.as_os_str(), "working directory")?.to_owned());
    request.environment = Some(bootstrap_environment.into_iter().collect());
    request.inherit_default_environment = Some(false);
    request.timeout_ms = payload
        .policy
        .timeout_ms
        .map(u32::try_from)
        .transpose()
        .map_err(invalid)?;
    Ok(request)
}

fn capture_temporary_root() -> io::Result<PathBuf> {
    std::env::temp_dir().canonicalize()
}

/// Check untrusted resource declarations before a trusted host broker reads or
/// copies them. Otherwise copying a reserved private file to an owned scratch
/// directory would erase the original path before the final mount validation.
/// Internal temporary-directory ownership is intentionally not accepted here.
pub fn validate_resource_paths(paths: &[PathBuf]) -> io::Result<()> {
    for path in paths {
        if !path.is_absolute() {
            return Err(invalid("sandbox resource path must be absolute"));
        }
    }
    reject_shared_temporary_grants(paths, &capture_temporary_root()?, &[])
}

/// Full settings may grant broad roots. The launcher partitions those roots;
/// explicit references to another task's private directory remain forbidden.
pub fn validate_configured_resource_paths(paths: &[PathBuf]) -> io::Result<()> {
    if paths.iter().any(|path| !path.is_absolute()) {
        return Err(invalid("sandbox resource path must be absolute"));
    }
    check_temporary_grants(paths, &capture_temporary_root()?, &[], true)
}

const PRIVATE_TEMP_PREFIXES: &[&str] = &[
    "agena-sandbox-",
    "agena-tool-worker-",
    "agena-media-worker-",
    "agena-native-worker-",
];

fn reserved_temporary_child(path: &Path, root: &Path) -> bool {
    identity_within(path, root)
        && path
            .components()
            .nth(root.components().count())
            .and_then(|component| component.as_os_str().to_str())
            .is_some_and(|name| {
                let name = name.to_ascii_lowercase();
                PRIVATE_TEMP_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            })
}

fn reject_shared_temporary_grants(
    grants: &[PathBuf],
    temporary_root: &Path,
    private_paths: &[PathBuf],
) -> io::Result<()> {
    check_temporary_grants(grants, temporary_root, private_paths, false)
}

fn shared_temporary_roots(temporary_root: &Path) -> io::Result<Vec<PathBuf>> {
    let shared_roots = vec![temporary_root.to_path_buf()];
    #[cfg(unix)]
    let shared_roots = {
        let mut roots = shared_roots;
        roots.extend([
            PathBuf::from("/tmp"),
            PathBuf::from("/var/tmp"),
            PathBuf::from("/private/tmp"),
        ]);
        roots
    };
    path_aliases::identities(&shared_roots)
}

fn check_temporary_grants(
    grants: &[PathBuf],
    temporary_root: &Path,
    private_paths: &[PathBuf],
    allow_shared_ancestors: bool,
) -> io::Result<()> {
    let roots = shared_temporary_roots(temporary_root)?;
    let mut owned = Vec::new();
    for path in private_paths {
        let identity = path.canonicalize()?;
        if !identity.is_dir()
            || !roots.iter().any(|root| {
                reserved_temporary_child(&identity, root)
                    && identity.components().count() == root.components().count() + 1
            })
        {
            return Err(invalid(
                "private_paths must name an existing reserved task directory directly inside a temporary root",
            ));
        }
        owned.push(identity);
    }
    for grant in grants {
        let identity = policy_path_identity(grant)?;
        if !allow_shared_ancestors
            && roots
                .iter()
                .any(|root| identity_within(root, &identity) || identity_within(root, grant))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "sandbox grant covers a shared temporary root: {}; use a task-specific subdirectory so other jobs' private files remain isolated",
                    grant.display()
                ),
            ));
        }
        let reserved = roots.iter().any(|root| {
            reserved_temporary_child(&identity, root) || reserved_temporary_child(grant, root)
        });
        if reserved && !owned.iter().any(|root| identity_within(&identity, root)) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "sandbox grant targets another task's private directory: {}; use an ordinary task directory or trusted private_paths ownership",
                    grant.display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
fn validate_policy(policy: &SandboxPolicy) -> io::Result<()> {
    validate_policy_for_root(policy, &capture_temporary_root()?)
}

fn validate_policy_for_root(policy: &SandboxPolicy, temporary_root: &Path) -> io::Result<()> {
    if !temporary_root.is_absolute() {
        return Err(invalid("sandbox temporary root must be absolute"));
    }
    if !policy.default_denied_paths.is_empty() {
        return Err(invalid(
            "configured default path denials must be compiled before launching MXC",
        ));
    }
    if let Some(timeout) = policy.timeout_ms
        && (timeout == 0 || timeout > u64::from(u32::MAX))
    {
        return Err(invalid(
            "sandbox timeout must be between 1 and 4294967295 milliseconds",
        ));
    }
    for path in policy
        .readonly_paths
        .iter()
        .chain(&policy.readwrite_paths)
        .chain(&policy.denied_paths)
        .chain(&policy.default_denied_paths)
        .chain(&policy.private_storage_roots)
        .chain(&policy.runtime_paths)
        .chain(&policy.private_paths)
    {
        if !path.is_absolute() || path.as_os_str().is_empty() {
            return Err(invalid(format!(
                "sandbox path must be absolute: {}",
                path.display()
            )));
        }
        utf8(path.as_os_str(), "policy path")?;
    }
    let granted = policy
        .readonly_paths
        .iter()
        .chain(&policy.readwrite_paths)
        .chain(&policy.runtime_paths)
        .cloned()
        .collect::<Vec<_>>();
    reject_shared_temporary_grants(&granted, temporary_root, &policy.private_paths)?;
    if let Some(home) = &policy.workload_home
        && (!policy.private_paths.contains(home) || !granted.contains(home) || !home.is_dir())
    {
        return Err(invalid(
            "sandbox workload HOME must be an existing caller-owned private grant",
        ));
    }
    // A granted child below a denied parent has different semantics on the
    // supported OSes. Refuse it instead of accidentally re-exposing the child.
    for granted in policy
        .readonly_paths
        .iter()
        .chain(&policy.readwrite_paths)
        .chain(&policy.runtime_paths)
    {
        if policy
            .denied_paths
            .iter()
            .any(|denied| granted.starts_with(denied))
        {
            return Err(invalid(format!(
                "sandbox grant is inside a denied path: {}",
                granted.display()
            )));
        }
    }
    network_policy(&policy.network)?;
    Ok(())
}

fn reject_private_storage_dependencies(
    policy: &SandboxPolicy,
    dependencies: &[PathBuf],
) -> io::Result<()> {
    if policy.private_storage_roots.is_empty() {
        return Ok(());
    }
    let private = path_aliases::identities(&policy.private_storage_roots)?;
    let mut approved = path_aliases::identities(&policy.readonly_paths)?;
    approved.extend(path_aliases::identities(&policy.readwrite_paths)?);
    for path in path_aliases::identities(dependencies)? {
        let overlaps_private = private
            .iter()
            .any(|root| identity_within(&path, root) || identity_within(root, &path));
        if overlaps_private && !approved.iter().any(|root| identity_within(&path, root)) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "sandbox runtime dependency overlaps Agena private storage: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn reject_denied_runtime(policy: &SandboxPolicy, granted: &[PathBuf]) -> io::Result<()> {
    let denied = policy
        .denied_paths
        .iter()
        .map(|path| policy_path_identity(path))
        .collect::<io::Result<Vec<_>>>()?;
    for path in granted {
        let identity = policy_path_identity(path)?;
        if denied
            .iter()
            .any(|denied| identity_within(&identity, denied))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "sandbox grant resolves inside a denied path: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

// MXC resolves aliases before constructing mounts. Compare the same identity
// here so a more-specific grant through a symlink cannot reopen a denied tree
// on Bubblewrap or ProcessContainer. Retain original spellings for MXC's alias
// reconstruction; this helper is only for validation.
pub(crate) fn policy_path_identity(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match current.canonicalize() {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                let mut normalized = PathBuf::new();
                for component in canonical.components() {
                    match component {
                        std::path::Component::CurDir => {}
                        std::path::Component::ParentDir => {
                            normalized.pop();
                        }
                        component => normalized.push(component.as_os_str()),
                    }
                }
                return Ok(normalized);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(name) = current.file_name().map(OsStr::to_os_string) else {
                    return Err(error);
                };
                let Some(parent) = current.parent() else {
                    return Err(error);
                };
                missing.push(name);
                current = parent.to_path_buf();
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn identity_within(path: &Path, parent: &Path) -> bool {
    #[cfg(not(windows))]
    {
        path.starts_with(parent)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
        let mut components = path.components();
        parent.components().all(|expected| {
            let Some(actual) = components.next() else {
                return false;
            };
            let actual: Vec<u16> = actual.as_os_str().encode_wide().collect();
            let expected: Vec<u16> = expected.as_os_str().encode_wide().collect();
            let (Ok(actual_len), Ok(expected_len)) =
                (i32::try_from(actual.len()), i32::try_from(expected.len()))
            else {
                return false;
            };
            // Paths are normalized, and Windows object names use ordinal,
            // case-insensitive comparison rather than locale-sensitive casing.
            unsafe {
                CompareStringOrdinal(
                    actual.as_ptr(),
                    actual_len,
                    expected.as_ptr(),
                    expected_len,
                    1,
                ) == CSTR_EQUAL
            }
        })
    }
}

fn path_strings(paths: Vec<PathBuf>) -> io::Result<Vec<String>> {
    let mut paths: Vec<_> = paths
        .iter()
        .map(|path| utf8(path.as_os_str(), "policy path").map(str::to_owned))
        .collect::<io::Result<_>>()?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Validate native network semantics before persistence or launch.
///
/// Host-specific Windows capabilities are checked by MXC when provisioning.
pub fn validate_network_config_for_platform(
    config: &NetworkPermissionConfig,
    platform: &str,
) -> io::Result<()> {
    config.validate_for_platform(platform).map_err(invalid)
}

fn network_policy(network: &SandboxNetwork) -> io::Result<NetworkPolicy> {
    let offline = NetworkPermissionConfig::offline();
    let config = match network {
        SandboxNetwork::Disabled => &offline,
        SandboxNetwork::Native { policy } => policy,
    };
    validate_network_config_for_platform(config, std::env::consts::OS)?;
    let config = config.simplified_for_sandbox().map_err(invalid)?;
    let egress = config.egress.as_ref();
    let ingress = config.ingress.as_ref();
    Ok(NetworkPolicy {
        egress: Some(NetworkEgressPolicy {
            default: Some(network_action(egress.and_then(|egress| egress.default))),
            allow: egress.and_then(|egress| {
                egress
                    .allow
                    .as_ref()
                    .map(|rules| rules.iter().map(network_rule).collect())
            }),
            deny: egress.and_then(|egress| {
                egress
                    .deny
                    .as_ref()
                    .map(|rules| rules.iter().map(network_rule).collect())
            }),
        }),
        ingress: Some(NetworkIngressPolicy {
            default: Some(network_action(ingress.and_then(|ingress| ingress.default))),
            host_loopback: Some(network_action(
                ingress.and_then(|ingress| ingress.host_loopback),
            )),
        }),
        runtime_config: None,
    })
}

fn network_action(action: Option<ConfigNetworkAction>) -> NetworkAction {
    match action.unwrap_or(ConfigNetworkAction::Deny) {
        ConfigNetworkAction::Allow => NetworkAction::Allow,
        ConfigNetworkAction::Deny => NetworkAction::Deny,
    }
}

fn network_rule(rule: &NetworkRuleConfig) -> NetworkRulePolicy {
    NetworkRulePolicy {
        to: rule.to.as_ref().map(|peers| {
            peers
                .iter()
                .map(|peer| NetworkPeerPolicy {
                    cidr: peer.cidr.clone(),
                    except: (!peer.except.is_empty()).then(|| peer.except.clone()),
                })
                .collect()
        }),
        ports: rule.ports.as_ref().map(|ports| {
            ports
                .iter()
                .map(|port| NetworkPortPolicy {
                    protocol: port.protocol.map(|protocol| match protocol {
                        ConfigNetworkProtocol::Tcp => mxc_sdk::v1::NetworkProtocol::Tcp,
                        ConfigNetworkProtocol::Udp => mxc_sdk::v1::NetworkProtocol::Udp,
                        ConfigNetworkProtocol::Icmp => mxc_sdk::v1::NetworkProtocol::Icmp,
                        ConfigNetworkProtocol::Any => mxc_sdk::v1::NetworkProtocol::Any,
                    }),
                    port: port.port,
                    end_port: port.end_port,
                })
                .collect()
        }),
    }
}

fn private_environment(
    environment: &mut HashMap<String, String>,
    scratch: &Path,
) -> io::Result<()> {
    // Windows environment names are case-insensitive. Remove aliases before
    // installing private values so HashMap iteration cannot restore host paths.
    environment.retain(|name, _| {
        !matches!(
            name.to_ascii_uppercase().as_str(),
            "HOME"
                | "USERPROFILE"
                | "XDG_CACHE_HOME"
                | "XDG_CONFIG_HOME"
                | "XDG_DATA_HOME"
                | "XDG_STATE_HOME"
                | "TMPDIR"
                | "TMP"
                | "TEMP"
                | "CARGO_HOME"
                | "APPDATA"
                | "LOCALAPPDATA"
                | "PYTHONDONTWRITEBYTECODE"
                | "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "ALL_PROXY"
                | "FTP_PROXY"
                | "NO_PROXY"
        )
    });
    for (name, directory) in [
        ("HOME", "home"),
        ("USERPROFILE", "home"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("TMPDIR", "tmp"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
        ("CARGO_HOME", "cargo"),
        ("APPDATA", "config"),
        ("LOCALAPPDATA", "data"),
    ] {
        let path = scratch.join(directory);
        std::fs::create_dir_all(&path)?;
        environment.insert(
            name.to_owned(),
            utf8(path.as_os_str(), "private directory")?.to_owned(),
        );
    }
    environment.insert("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned());
    Ok(())
}

fn capture_rustup_home(source: &HashMap<String, String>, target: &mut HashMap<String, String>) {
    if target.contains_key("RUSTUP_HOME") {
        return;
    }
    let home = source.get("HOME").or_else(|| source.get("USERPROFILE"));
    if let Some(home) = home {
        let rustup = Path::new(home).join(".rustup");
        if rustup.join("toolchains").is_dir()
            && let Some(value) = rustup.to_str()
        {
            target.insert("RUSTUP_HOME".to_owned(), value.to_owned());
        }
    }
}

/// Discover executable files and installation subdirectories, never whole home
/// directories. Data paths in argv are intentionally not inferred as permissions.
fn runtime_paths(
    argv: &[String],
    cwd: &Path,
    environment: &HashMap<String, String>,
) -> io::Result<Vec<PathBuf>> {
    let program = argv.first().ok_or_else(|| invalid("empty sandbox argv"))?;
    let path = environment.get("PATH").or_else(|| environment.get("Path"));
    let executable = which::which_in(program, path, cwd).map_err(|error| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("sandbox executable {program:?} was not found: {error}"),
        )
    })?;
    let mut paths = Vec::new();
    add_executable_paths(&executable, &mut paths)?;
    if let Some(path) = path {
        for directory in std::env::split_paths(path) {
            if directory.is_absolute() && is_system_runtime_directory(&directory) {
                paths.push(directory.clone());
                add_installation_libraries(&directory, &mut paths);
            }
        }
    }
    for name in ["VIRTUAL_ENV", "CONDA_PREFIX"] {
        if let Some(value) = environment.get(name) {
            let prefix = Path::new(value);
            if prefix.is_absolute() && prefix.parent().is_some() && prefix.is_dir() {
                for name in ["bin/python", "Scripts/python.exe", "python.exe"] {
                    let executable = prefix.join(name);
                    if executable.is_file() {
                        add_executable_paths(&executable, &mut paths)?;
                        break;
                    }
                }
            }
        }
    }
    if let Some(value) = environment.get("RUSTUP_HOME") {
        let prefix = Path::new(value);
        for child in ["settings.toml", "toolchains"] {
            let path = prefix.join(child);
            if prefix.is_absolute() && path.exists() {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn add_executable_paths(executable: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    paths.push(executable.to_path_buf());
    let canonical = executable.canonicalize()?;
    paths.push(canonical.clone());
    for executable in [executable, canonical.as_path()] {
        if let Some(directory) = executable.parent() {
            add_installation_libraries(directory, paths);
            add_python_libraries(executable, paths)?;
            #[cfg(windows)]
            {
                // DLLs beside a configured executable are runtime files. Do not
                // turn its arbitrary parent into a blanket readable directory.
                for entry in std::fs::read_dir(directory)? {
                    let path = entry?.path();
                    if path
                        .extension()
                        .and_then(OsStr::to_str)
                        .is_some_and(|extension| {
                            matches!(
                                extension.to_ascii_lowercase().as_str(),
                                "dll" | "pyd" | "manifest"
                            )
                        })
                    {
                        paths.push(path);
                    }
                }
            }
        }
    }
    Ok(())
}

fn add_installation_libraries(directory: &Path, paths: &mut Vec<PathBuf>) {
    if !is_system_runtime_directory(directory) {
        return;
    }
    let Some(prefix) = directory.parent() else {
        return;
    };
    for child in ["lib", "lib64", "libexec", "share", "include"] {
        let path = prefix.join(child);
        if path.exists() {
            paths.push(path);
        }
    }
}

// A directory being named "bin" says nothing about its parent's data. In
// particular, ~/.local/share contains application stores, not just runtimes.
fn is_system_runtime_directory(directory: &Path) -> bool {
    let Ok(directory) = directory.canonicalize() else {
        return false;
    };
    [
        "/bin",
        "/sbin",
        "/usr/bin",
        "/usr/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
    ]
    .iter()
    .any(|known| directory == Path::new(known))
}

// Recognize the interpreter's own versioned standard-library/site-packages
// tree. Arbitrary adjacent lib/share/include folders require runtime_paths.
fn add_python_libraries(executable: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    let Some(name) = executable.file_stem().and_then(OsStr::to_str) else {
        return Ok(());
    };
    let name = name.to_ascii_lowercase();
    let Some(version) = name
        .strip_prefix("python")
        .or_else(|| name.strip_prefix("pypy"))
    else {
        return Ok(());
    };
    if !version
        .chars()
        .all(|character| character.is_ascii_digit() || character == '.' || character == 'w')
    {
        return Ok(());
    }
    let Some(directory) = executable.parent() else {
        return Ok(());
    };
    let prefix = if matches!(
        directory.file_name().and_then(OsStr::to_str),
        Some("bin" | "Scripts")
    ) {
        directory.parent().unwrap_or(directory)
    } else {
        directory
    };
    let configuration = prefix.join("pyvenv.cfg");
    // A user shim may point at a system interpreter. Its unrelated sibling
    // lib directory is not part of that interpreter; venvs identify themselves.
    if executable.canonicalize()? != executable && !configuration.is_file() {
        return Ok(());
    }
    if configuration.is_file() {
        paths.push(configuration);
    }
    for name in ["lib", "lib64"] {
        let directory = prefix.join(name);
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            let Some(version) = name.strip_prefix("python") else {
                continue;
            };
            if !version.is_empty()
                && version
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
                && path.is_dir()
                && (path.join("os.py").is_file() || path.join("site-packages").is_dir())
            {
                paths.push(path);
            }
        }
    }
    #[cfg(windows)]
    {
        let library = prefix.join("Lib");
        if library.join("os.py").is_file() {
            paths.push(library);
            let extensions = prefix.join("DLLs");
            if extensions.is_dir() {
                paths.push(extensions);
            }
        }
    }
    Ok(())
}

fn command_line(argv: &[String]) -> io::Result<String> {
    if argv.is_empty() || argv.iter().any(|argument| argument.contains('\0')) {
        return Err(invalid("sandbox argv must be nonempty and NUL-free"));
    }
    #[cfg(windows)]
    {
        Ok(argv
            .iter()
            .map(|argument| quote_windows(argument))
            .collect::<Vec<_>>()
            .join(" "))
    }
    #[cfg(not(windows))]
    {
        Ok(format!(
            "exec {}",
            argv.iter()
                .map(|argument| quote_posix(argument))
                .collect::<Vec<_>>()
                .join(" ")
        ))
    }
}

#[cfg(any(not(windows), test))]
fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(any(windows, test))]
fn quote_windows(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        match character {
            '\\' => slashes += 1,
            '"' => {
                result.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                result.push('"');
                slashes = 0;
            }
            _ => {
                result.extend(std::iter::repeat_n('\\', slashes));
                result.push(character);
                slashes = 0;
            }
        }
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}

fn prerequisite_hint() -> &'static str {
    if cfg!(target_os = "linux") {
        "Linux needs working unprivileged user namespaces and bubblewrap; network-enabled policies additionally need slirp4netns, iproute2 and iptables"
    } else if cfg!(target_os = "macos") {
        "The Seatbelt backend requires a supported macOS version"
    } else if cfg!(windows) {
        "Windows needs a supported PSEC ProcessContainer implementation with the requested filesystem/network capabilities"
    } else {
        "This platform does not have a supported native MXC process backend"
    }
}

fn utf8<'a>(value: &'a OsStr, kind: &str) -> io::Result<&'a str> {
    value
        .to_str()
        .ok_or_else(|| invalid(format!("sandbox {kind} is not valid UTF-8")))
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
}

#[cfg(unix)]
static TERMINATION_SIGNAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn termination_signal(signal: libc::c_int) {
    if signal != libc::SIGINT {
        TERMINATION_SIGNAL.store(signal, std::sync::atomic::Ordering::Relaxed);
    }
}

fn install_termination_handlers() -> io::Result<()> {
    #[cfg(unix)]
    {
        for signal in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
            // Only atomic stores occur in the handler. A caught disposition is
            // reset on exec, so the workload receives normal terminal signals.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = termination_signal as *const () as usize;
            unsafe { libc::sigemptyset(&mut action.sa_mask) };
            if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } == -1 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invocation_sandbox_mode_is_isolated_and_restored_after_awaits() {
        let ordinary = enabled();
        tokio::join!(
            scope_enabled(false, async {
                tokio::task::yield_now().await;
                assert!(!enabled());
            }),
            async {
                tokio::task::yield_now().await;
                assert_eq!(enabled(), ordinary);
            },
        );
        assert_eq!(enabled(), ordinary);
    }

    #[test]
    fn trusted_switch_defaults_to_isolation_and_process_setting_overrides_legacy_alias() {
        assert!(enabled_from_lookup(|_| None));
        for value in ["disabled", "false", "0", "off", " Disabled "] {
            assert!(!enabled_from_lookup(
                |key| (key == "AGENA_SANDBOX").then(|| value.into())
            ));
        }
        assert!(!enabled_from_lookup(
            |key| (key == "AGENA_SHELL_SANDBOX").then(|| "disabled".into())
        ));
        assert!(enabled_from_lookup(|key| Some(
            if key == "AGENA_SANDBOX" {
                "enabled"
            } else {
                "disabled"
            }
            .into()
        )));
        assert!(enabled_from_lookup(|_| Some("misspelled".into())));
    }

    #[test]
    fn direct_argv_keeps_environment_and_skips_mxc_policy_preparation() {
        let argv = vec!["program".into(), "a b".into()];
        let mut environment = HashMap::from([
            ("HOME".into(), "/actual/home".into()),
            ("AGENA_SANDBOX".into(), "enabled".into()),
        ]);
        let original = environment.clone();
        let policy = SandboxPolicy {
            disabled: true,
            readonly_paths: vec![PathBuf::from("relative-not-a-mount")],
            ..Default::default()
        };
        assert_eq!(
            wrap_argv(&argv, Path::new("."), &mut environment, &policy).unwrap(),
            argv
        );
        assert_eq!(environment, original);
        assert_ne!(policy, SandboxPolicy::default());
        assert!(
            !serde_json::from_str::<SandboxPolicy>("{}")
                .unwrap()
                .disabled
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_commands_keep_cwd_environment_and_preconfigured_stdio() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("input"), "host-file").unwrap();
        let policy = SandboxPolicy {
            disabled: true,
            denied_paths: vec![dir.path().to_path_buf()],
            ..Default::default()
        };
        let args = vec![
            "-c".into(),
            "cat input; printf '%s' \"$AGENA_DIRECT_TEST_MARKER\"".into(),
        ];
        let env = HashMap::from([("AGENA_DIRECT_TEST_MARKER".into(), "-direct".into())]);
        let output = command("/bin/sh", &args, dir.path(), &env, &policy)
            .unwrap()
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"host-file-direct");

        let mut standard = std::process::Command::new("/bin/sh");
        standard
            .args(&args)
            .current_dir(dir.path())
            .env_clear()
            .envs(&env)
            .stdout(std::process::Stdio::piped());
        wrap_std_command(&mut standard, &policy).unwrap();
        let output = standard.spawn().unwrap().wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"host-file-direct");

        let mut asynchronous = tokio::process::Command::new("/bin/sh");
        asynchronous
            .args(&args)
            .current_dir(dir.path())
            .env_clear()
            .envs(&env)
            .stdout(std::process::Stdio::piped());
        wrap_tokio_command(&mut asynchronous, &policy).unwrap();
        let output = asynchronous
            .spawn()
            .unwrap()
            .wait_with_output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"host-file-direct");
    }

    #[test]
    fn parent_credentials_and_proxy_overrides_are_not_inherited() {
        let input = HashMap::from([
            ("PATH".into(), "/usr/bin".into()),
            ("LANG".into(), "en_US.UTF-8".into()),
            ("OPENAI_API_KEY".into(), "secret".into()),
            ("AWS_SESSION_TOKEN".into(), "secret".into()),
            ("SSH_AUTH_SOCK".into(), "/tmp/agent.sock".into()),
            ("HTTP_PROXY".into(), "http://secret-proxy".into()),
            ("LD_PRELOAD".into(), "/tmp/inject.so".into()),
        ]);
        let safe = sanitize_environment(&input);
        assert_eq!(safe.len(), 2);
        assert_eq!(safe.get("PATH").unwrap(), "/usr/bin");
    }

    #[test]
    fn explicit_loader_injection_is_rejected_before_a_process_can_start() {
        for name in [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "LD_AUDIT",
            "DYLD_INSERT_LIBRARIES",
            "GLIBC_TUNABLES",
        ] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.env(name, "loader-secret-value");
            let error = wrap_std_command(&mut command, &SandboxPolicy::default()).unwrap_err();
            assert!(error.to_string().contains(name));
            assert!(!error.to_string().contains("loader-secret-value"));
        }
    }

    #[test]
    fn workload_native_settings_are_inert_until_the_mxc_payload() {
        let directory = tempfile::tempdir().unwrap();
        let trusted = trusted_launcher_environment();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.current_dir(directory.path());
        for name in [
            "PATH",
            "SystemRoot",
            "COMSPEC",
            "PATHEXT",
            "MXC_BWRAP_PATH",
            "EXPLICIT_API_TOKEN",
        ] {
            command.env(name, format!("workload-value-{name}"));
        }
        wrap_std_command(&mut command, &SandboxPolicy::default()).unwrap();
        let environment: HashMap<_, _> = command
            .get_envs()
            .filter_map(|(name, value)| {
                Some((name.to_str()?.to_owned(), value?.to_str()?.to_owned()))
            })
            .collect();
        for name in [
            "PATH",
            "SystemRoot",
            "COMSPEC",
            "PATHEXT",
            "MXC_BWRAP_PATH",
            "EXPLICIT_API_TOKEN",
        ] {
            let key = environment_key(name);
            assert_eq!(environment.get(&key), trusted.get(&key), "{name}");
            assert!(
                environment
                    .iter()
                    .any(|(key, value)| key.starts_with(PAYLOAD_ENV_PREFIX)
                        && value == &format!("workload-value-{name}"))
            );
        }
        let encoded = command.get_args().nth(1).unwrap().to_str().unwrap();
        let bytes = URL_SAFE_NO_PAD.decode(encoded).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("workload-value-"));
    }

    #[test]
    fn writable_resources_cannot_replace_launcher_or_backend_lookup_paths() {
        let executable = launcher_executable().unwrap();
        let cwd = std::env::current_dir().unwrap();
        let mut policy = SandboxPolicy {
            readwrite_paths: vec![executable.parent().unwrap().to_path_buf()],
            ..Default::default()
        };
        let mut environment = HashMap::new();
        let error = wrap_argv(
            &["unused-program".to_owned()],
            &cwd,
            &mut environment,
            &policy,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            error
                .to_string()
                .contains("trusted sandbox launcher/backend")
        );

        let root = tempfile::tempdir().unwrap();
        let backend = root.path().join("backend-bin");
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&backend).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let native = HashMap::from([("PATH".to_owned(), backend.to_str().unwrap().to_owned())]);
        for grant in [&backend, &backend.join("bwrap")] {
            policy.readwrite_paths = vec![grant.to_path_buf()];
            assert!(reject_writable_launcher_paths(&policy, &executable, &cwd, &native).is_err());
        }
        #[cfg(unix)]
        {
            let alias = root.path().join("launcher-directory");
            std::os::unix::fs::symlink(executable.parent().unwrap(), &alias).unwrap();
            policy.readwrite_paths = vec![alias];
            assert!(reject_writable_launcher_paths(&policy, &executable, &cwd, &native).is_err());
        }
        policy.readonly_paths = vec![executable.parent().unwrap().to_path_buf(), backend];
        policy.readwrite_paths = vec![workspace];
        reject_writable_launcher_paths(&policy, &executable, &cwd, &native).unwrap();
    }

    #[test]
    fn timeout_and_cross_platform_deny_conflicts_fail_before_spawn() {
        let mut policy = SandboxPolicy {
            timeout_ms: Some(u64::from(u32::MAX) + 1),
            ..Default::default()
        };
        assert!(validate_policy(&policy).is_err());
        policy.timeout_ms = None;
        let root = std::env::temp_dir();
        policy.denied_paths = vec![root.join("secret")];
        policy.readonly_paths = vec![root.join("secret/child")];
        assert!(validate_policy(&policy).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn runtime_symlink_alias_cannot_regrant_a_denied_directory() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let secret = root.path().join("secret");
        std::fs::create_dir_all(secret.join("bin")).unwrap();
        let alias = root.path().join("runtime");
        symlink(&secret, &alias).unwrap();
        let policy = SandboxPolicy {
            denied_paths: vec![secret],
            ..Default::default()
        };
        assert!(reject_denied_runtime(&policy, &[alias.join("bin")]).is_err());
        assert!(reject_denied_runtime(&policy, &[alias.join("not-created")]).is_err());
    }

    #[test]
    fn native_network_rejects_domains_and_invalid_port_ranges() {
        for value in [
            serde_json::json!({"allow":[{"to":"example.com"}]}),
            serde_json::json!({"allow":[{"to":"192.0.2.1","protocol":"tcp","ports":["443-80"]}]}),
            serde_json::json!({"allow":[{"to":"192.0.2.0/24","except":["198.51.100.0/24"]}]}),
        ] {
            assert!(serde_json::from_value::<NetworkPermissionConfig>(value).is_err());
        }
    }

    #[test]
    fn native_network_enforces_backend_capabilities_before_launch() {
        let selective = serde_json::from_value(serde_json::json!({
            "default": "deny", "allow": [{"to": "192.0.2.1"}]
        }))
        .unwrap();
        assert!(validate_network_config_for_platform(&selective, "linux").is_ok());
        assert!(validate_network_config_for_platform(&selective, "windows").is_ok());
        assert!(validate_network_config_for_platform(&selective, "macos").is_err());
        let loopback = serde_json::from_value(serde_json::json!({
            "inbound": "deny", "hostLoopback": "allow"
        }))
        .unwrap();
        assert!(validate_network_config_for_platform(&loopback, "linux").is_err());
        assert!(validate_network_config_for_platform(&loopback, "macos").is_err());
        assert!(validate_network_config_for_platform(&loopback, "windows").is_ok());
        assert!(
            validate_network_config_for_platform(&NetworkPermissionConfig::offline(), "macos")
                .is_ok()
        );
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn native_network_preserves_cidr_exceptions_protocols_ranges_and_denies() {
        let config = serde_json::from_value(serde_json::json!({
            "default": "deny",
            "allow": [
                {"to":"10.20.0.0/16","except":["10.20.10.0/24"],"protocol":"tcp","ports":["8000-8099"]},
                {"to":"fd20:20::/48","except":["fd20:20:0:10::/64"],"protocol":"icmp"},
                {"to":"*","protocol":"udp","ports":[53]},
                {"to":"192.0.2.53","protocol":"udp","ports":[53]}
            ],
            "deny": [{"to":"10.20.66.0/24"}]
        })).unwrap();
        let policy = network_policy(&SandboxNetwork::Native { policy: config }).unwrap();
        assert!(policy.runtime_config.is_none());
        let egress = policy.egress.unwrap();
        assert!(matches!(egress.default, Some(NetworkAction::Deny)));
        let allow = egress.allow.unwrap();
        assert_eq!(allow.len(), 4);
        assert_eq!(
            allow[0].to.as_ref().unwrap()[0].except.as_ref().unwrap(),
            &["10.20.10.0/24"]
        );
        assert_eq!(allow[0].ports.as_ref().unwrap()[0].end_port, Some(8099));
        assert_eq!(allow[1].to.as_ref().unwrap()[0].cidr, "fd20:20::/48");
        assert!(matches!(
            allow[1].ports.as_ref().unwrap()[0].protocol,
            Some(mxc_sdk::v1::NetworkProtocol::Icmp)
        ));
        assert!(allow[1].ports.as_ref().unwrap()[0].port.is_none());
        assert!(allow[2].to.is_none());
        assert!(matches!(
            allow[2].ports.as_ref().unwrap()[0].protocol,
            Some(mxc_sdk::v1::NetworkProtocol::Udp)
        ));
        assert_eq!(allow[3].to.as_ref().unwrap()[0].cidr, "192.0.2.53/32");
        let deny = egress.deny.unwrap();
        assert!(deny[0].ports.is_none());
        assert_eq!(deny[0].to.as_ref().unwrap()[0].cidr, "10.20.66.0/24");
        assert!(matches!(
            policy.ingress.unwrap().host_loopback,
            Some(NetworkAction::Deny)
        ));
    }

    #[test]
    fn native_network_redundant_rules_are_lowered_before_mxc_mapping() {
        for (value, allowed) in [
            (
                serde_json::json!({"default":"allow","allow":[{"to":"192.0.2.1","ports":[443]}]}),
                true,
            ),
            (
                serde_json::json!({"default":"deny","deny":[{"to":"192.0.2.1"}]}),
                false,
            ),
            (serde_json::json!({"allow":[{"to":"*"}]}), true),
            (
                serde_json::json!({"allow":[{"to":"192.0.2.1"}],"deny":[{"to":"*"}]}),
                false,
            ),
        ] {
            let config = serde_json::from_value(value).unwrap();
            let policy = network_policy(&SandboxNetwork::Native { policy: config }).unwrap();
            let egress = policy.egress.unwrap();
            assert_eq!(
                matches!(egress.default, Some(NetworkAction::Allow)),
                allowed
            );
            assert!(egress.allow.unwrap_or_default().is_empty());
            assert!(egress.deny.unwrap_or_default().is_empty());
        }
    }

    #[test]
    fn native_network_defaults_to_deny_without_runtime_transport() {
        let policy = network_policy(&SandboxNetwork::Native {
            policy: NetworkPermissionConfig::default(),
        })
        .unwrap();
        assert!(matches!(
            policy.egress.unwrap().default,
            Some(NetworkAction::Deny)
        ));
        let ingress = policy.ingress.unwrap();
        assert!(matches!(ingress.default, Some(NetworkAction::Deny)));
        assert!(matches!(ingress.host_loopback, Some(NetworkAction::Deny)));
        assert!(policy.runtime_config.is_none());
    }

    #[test]
    fn user_runtime_layout_does_not_expose_application_data_or_the_repository() {
        let directory = tempfile::tempdir().unwrap();
        let prefix = directory.path().join(".local");
        let binary = prefix.join("bin/python3");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"test executable").unwrap();
        for name in ["share/agena", "include", "lib/private"] {
            std::fs::create_dir_all(prefix.join(name)).unwrap();
        }
        let mut granted = Vec::new();
        add_executable_paths(&binary, &mut granted).unwrap();
        let private_data = prefix.join("share/agena/credentials");
        assert!(!granted.iter().any(|path| private_data.starts_with(path)));
        assert!(!granted.contains(&prefix.join("lib")));
        assert!(!granted.contains(&prefix.join("include")));
        let build = directory.path().join("repository/target/debug/agena");
        std::fs::create_dir_all(build.parent().unwrap()).unwrap();
        std::fs::write(&build, b"test executable").unwrap();
        granted.clear();
        add_executable_paths(&build, &mut granted).unwrap();
        assert!(
            granted
                .iter()
                .all(|path| path == &build || path == &build.canonicalize().unwrap())
        );
        let environment = HashMap::from([(
            "PATH".to_owned(),
            binary.parent().unwrap().display().to_string(),
        )]);
        let granted = runtime_paths(
            &[std::env::current_exe().unwrap().display().to_string()],
            directory.path(),
            &environment,
        )
        .unwrap();
        assert!(!granted.contains(&prefix.join("bin")));
        assert!(!granted.iter().any(|path| private_data.starts_with(path)));
    }

    #[test]
    fn reserved_temporary_directories_require_trusted_ownership() {
        let root = capture_temporary_root().unwrap();
        for prefix in PRIVATE_TEMP_PREFIXES {
            let directory = tempfile::Builder::new()
                .prefix(prefix)
                .tempdir_in(&root)
                .unwrap();
            let file = directory.path().join("workload.json");
            std::fs::write(&file, b"private").unwrap();
            let mut policy = SandboxPolicy {
                readonly_paths: vec![file.clone()],
                ..Default::default()
            };
            assert_eq!(
                validate_policy(&policy).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            policy.private_paths.push(directory.path().to_path_buf());
            validate_policy(&policy).unwrap();
            policy.readonly_paths = vec![root.clone()];
            assert_eq!(
                validate_policy(&policy).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn reserved_temporary_paths_and_their_symlink_aliases_stay_private() {
        use std::os::unix::fs::symlink;
        let root = capture_temporary_root().unwrap();
        let private = tempfile::Builder::new()
            .prefix("agena-sandbox-")
            .tempdir_in(&root)
            .unwrap();
        let ordinary = tempfile::tempdir_in(&root).unwrap();
        let alias = ordinary.path().join("alias");
        symlink(private.path(), &alias).unwrap();
        let policy = SandboxPolicy {
            readonly_paths: vec![alias],
            ..Default::default()
        };
        assert_eq!(
            validate_policy(&policy).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let reserved_alias = private.path().join("redirect");
        symlink(ordinary.path(), &reserved_alias).unwrap();
        let policy = SandboxPolicy {
            readonly_paths: vec![reserved_alias],
            ..Default::default()
        };
        assert_eq!(
            validate_policy(&policy).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(windows)]
    #[test]
    fn reserved_temporary_namespace_comparison_ignores_windows_case() {
        assert!(reserved_temporary_child(
            Path::new(r"C:\TEMP\AGENA-SANDBOX-TEST\workload.json"),
            Path::new(r"c:\temp"),
        ));
    }

    #[test]
    fn credentials_cross_the_mxc_boundary_only_in_a_private_payload_file() {
        let directory = tempfile::tempdir().unwrap();
        let environment = HashMap::from([
            ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
            (
                "EXPLICIT_API_TOKEN".into(),
                "trusted-override-secret".into(),
            ),
        ]);
        let payload = LauncherRequest {
            argv: vec![std::env::current_exe().unwrap().display().to_string()],
            cwd: directory.path().to_path_buf(),
            temporary_root: capture_temporary_root().unwrap(),
            environment_keys: environment.keys().cloned().collect(),
            environment,
            policy: SandboxPolicy {
                private_build_cache: true,
                ..Default::default()
            },
        };
        let transported = serde_json::to_string(&payload).unwrap();
        assert!(!transported.contains("trusted-override-secret"));
        let request = mxc_request(&payload, directory.path()).unwrap();
        assert!(!request.command.contains("trusted-override-secret"));
        for (_, value) in request.environment.unwrap() {
            assert!(!value.contains("trusted-override-secret"));
        }
        assert!(request.network.unwrap().runtime_config.is_none());
        let file = directory.path().join("workload.json");
        let workload: WorkloadPayload =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(
            workload.environment["EXPLICIT_API_TOKEN"],
            "trusted-override-secret"
        );
        assert_eq!(
            Path::new(&workload.environment["CARGO_TARGET_DIR"]),
            directory.path().join("cache/cargo-target")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn shared_temporary_roots_are_rejected_but_task_subdirectories_are_allowed() {
        let root = capture_temporary_root().unwrap();
        let directory = tempfile::tempdir_in(&root).unwrap();
        for policy in [
            SandboxPolicy {
                readonly_paths: vec![root.clone()],
                ..Default::default()
            },
            SandboxPolicy {
                readwrite_paths: vec![root.clone()],
                ..Default::default()
            },
            SandboxPolicy {
                runtime_paths: vec![root.clone()],
                ..Default::default()
            },
        ] {
            assert_eq!(
                validate_policy(&policy).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
        validate_policy(&SandboxPolicy {
            readonly_paths: vec![directory.path().to_path_buf()],
            readwrite_paths: vec![directory.path().to_path_buf()],
            runtime_paths: vec![directory.path().to_path_buf()],
            ..Default::default()
        })
        .unwrap();
        let parent = root.parent().unwrap();
        assert!(reject_shared_temporary_grants(&[parent.to_path_buf()], &root, &[]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn posix_argv_survives_shell_metacharacters_without_evaluation() {
        let arguments = [
            "",
            "a'b",
            "two words",
            "$(printf injected)",
            "x; printf injected",
            "line\nbreak",
            r"C:\a\b",
        ];
        let mut argv = vec!["printf".to_owned(), "%s\\0".to_owned()];
        argv.extend(arguments.iter().map(|value| (*value).to_owned()));
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command_line(&argv).unwrap())
            .output()
            .unwrap();
        assert!(output.status.success());
        let expected: Vec<u8> = arguments
            .iter()
            .flat_map(|value| value.bytes().chain(std::iter::once(0)))
            .collect();
        assert_eq!(output.stdout, expected);
    }

    #[test]
    fn windows_argv_quotes_empty_arguments_quotes_and_trailing_backslashes() {
        assert_eq!(quote_windows(""), "\"\"");
        assert_eq!(quote_windows("a b"), "\"a b\"");
        assert_eq!(quote_windows("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_windows("a\\"), "\"a\\\\\"");
    }

    #[test]
    fn runtime_dependencies_cannot_reopen_hidden_instance_storage() {
        let fixture = tempfile::tempdir().unwrap();
        let instance = fixture.path().canonicalize().unwrap();
        let workspaces = instance.join("workspaces");
        let state = workspaces.join("own");
        let foreign = workspaces.join("foreign");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir(&foreign).unwrap();
        let policy = SandboxPolicy {
            private_storage_roots: vec![instance.join("secrets"), workspaces],
            readwrite_paths: vec![state.clone()],
            ..Default::default()
        };
        assert!(reject_private_storage_dependencies(&policy, &[state]).is_ok());
        for blocked in [foreign, instance] {
            let error = reject_private_storage_dependencies(&policy, &[blocked]).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        }
    }

    #[test]
    fn private_environment_cannot_reuse_the_host_home_or_temp() {
        let directory = tempfile::tempdir().unwrap();
        let mut environment = HashMap::from([
            ("HOME".into(), "/secret/home".into()),
            ("TMPDIR".into(), "/shared/tmp".into()),
            ("Home".into(), "/secret/home".into()),
            ("Temp".into(), "/shared/tmp".into()),
            ("Http_Proxy".into(), "http://secret-proxy".into()),
        ]);
        private_environment(&mut environment, directory.path()).unwrap();
        assert!(!environment.contains_key("Home"));
        assert!(!environment.contains_key("Temp"));
        assert!(!environment.contains_key("Http_Proxy"));
        assert!(Path::new(&environment["HOME"]).starts_with(directory.path()));
        assert!(Path::new(&environment["TMPDIR"]).starts_with(directory.path()));
        assert!(Path::new(&environment["CARGO_HOME"]).starts_with(directory.path()));
    }
}
