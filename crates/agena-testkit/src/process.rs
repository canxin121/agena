//! Ownership of one foreground product process and its bounded output readers.
//!
//! ManagedChild owns a Unix process session or Windows Job Object. On Unix,
//! descendants that create a new session need their own fixture ownership or
//! the product's graceful teardown; this module is not an OS containment layer.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use agena_process::ManagedChild;
use anyhow::{Context, Result, anyhow, bail, ensure};
use tokio::{io::AsyncReadExt, process::Command, task::JoinHandle};

use crate::config::CasePaths;

#[cfg(target_os = "linux")]
mod descendants;

pub(crate) const LOG_LIMIT: usize = 256 * 1024;

#[derive(Clone)]
pub(crate) struct ProcessSpec {
    pub binary: PathBuf,
    pub raw_args: Vec<OsString>,
    pub environment: BTreeMap<OsString, OsString>,
    pub password: Option<String>,
}

impl ProcessSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.binary.is_file(),
            "Agena binary does not exist: {}",
            self.binary.display()
        );
        for arg in &self.raw_args {
            let arg = arg.to_string_lossy();
            let name = arg.split('=').next().unwrap_or_default();
            ensure!(
                !matches!(
                    name,
                    "--data-dir"
                        | "--host"
                        | "--port"
                        | "--workspace"
                        | "--password"
                        | "--"
                        | "server"
                        | "start"
                        | "stop"
                        | "install"
                        | "uninstall"
                        | "restart"
                        | "status"
                ),
                "raw CLI argument {name:?} changes process ownership or invokes server lifecycle; use the fixture's dedicated inputs"
            );
        }
        Ok(())
    }

    pub fn command(&self, paths: &CasePaths) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .arg("--data-dir")
            .arg(&paths.data_dir)
            .arg("server")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg("0")
            .arg("--workspace")
            .arg(&paths.workspace)
            .args(&self.raw_args)
            .current_dir(&paths.cwd)
            .env_clear()
            .envs(&self.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command
            .env("TMUX_TMPDIR", paths.root.join("tmux"))
            .env_remove("TMUX");
        command
    }
}

pub(crate) fn isolated_environment(
    paths: &CasePaths,
    password: Option<&str>,
    overrides: &BTreeMap<OsString, Option<OsString>>,
) -> BTreeMap<OsString, OsString> {
    let mut environment = BTreeMap::new();
    // Keep executable lookup and Windows process-loader necessities, not user
    // credentials, Agena settings, proxy variables, or provider API keys.
    for name in ["PATH", "SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(OsString::from(name), value);
        }
    }
    for (name, value) in [
        ("HOME", paths.home.clone()),
        ("USERPROFILE", paths.home.clone()),
        ("XDG_CONFIG_HOME", paths.home.join(".config")),
        ("XDG_CACHE_HOME", paths.home.join(".cache")),
        ("XDG_DATA_HOME", paths.home.join(".local/share")),
        ("APPDATA", paths.home.join("AppData/Roaming")),
        ("LOCALAPPDATA", paths.home.join("AppData/Local")),
        ("TMPDIR", paths.root.join("tmp")),
        ("TMP", paths.root.join("tmp")),
        ("TEMP", paths.root.join("tmp")),
    ] {
        environment.insert(OsString::from(name), value.into_os_string());
    }
    environment.insert(
        OsString::from("AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES"),
        OsString::from("1"),
    );
    environment.insert(
        OsString::from("NO_PROXY"),
        OsString::from("127.0.0.1,localhost,::1"),
    );
    if let Some(password) = password {
        environment.insert(
            OsString::from("AGENA_SERVER_PASSWORD"),
            OsString::from(password),
        );
    }
    for (name, value) in overrides {
        if let Some(value) = value {
            environment.insert(name.clone(), value.clone());
        } else {
            environment.remove(name.as_os_str());
        }
    }
    // A tmux server daemonizes and leaves the Agena process tree. Unless its
    // socket is isolated, a test could attach to the developer's real tmux
    // server and leave PTY shells behind after SIGKILL. This ownership setting
    // is protected from user-supplied fixture environment overrides.
    #[cfg(unix)]
    {
        environment.remove(OsStr::new("TMUX"));
        environment.insert(
            OsString::from("TMUX_TMPDIR"),
            paths.root.join("tmux").into_os_string(),
        );
    }
    environment
}

pub(crate) fn environment_password(
    environment: &BTreeMap<OsString, OsString>,
) -> Result<Option<String>> {
    environment
        .get(OsStr::new("AGENA_SERVER_PASSWORD"))
        .map(|value| {
            value
                .clone()
                .into_string()
                .map_err(|_| anyhow!("AGENA_SERVER_PASSWORD must be UTF-8"))
        })
        .transpose()
}

/// On Unix this means SIGTERM followed by a successful exit. Windows currently
/// uses ManagedChild's Job Object kill, and is deliberately reported as forced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownKind {
    Graceful,
    Forced,
    Crash,
    AlreadyExited,
}

#[derive(Clone, Debug)]
pub struct ProcessExit {
    pub kind: ShutdownKind,
    pub code: Option<i32>,
    pub status: String,
}

#[derive(Default)]
struct Tail {
    bytes: Vec<u8>,
    total_bytes: u64,
}

impl Tail {
    fn append(&mut self, bytes: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len() as u64);
        if bytes.len() >= LOG_LIMIT {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&bytes[bytes.len() - LOG_LIMIT..]);
        } else {
            let excess = self
                .bytes
                .len()
                .saturating_add(bytes.len())
                .saturating_sub(LOG_LIMIT);
            if excess != 0 {
                self.bytes.drain(..excess);
            }
            self.bytes.extend_from_slice(bytes);
        }
    }

    fn text(&self) -> String {
        let truncated = self.total_bytes.saturating_sub(self.bytes.len() as u64);
        if truncated == 0 {
            String::from_utf8_lossy(&self.bytes).into_owned()
        } else {
            format!(
                "[{} earlier bytes omitted; final {} bytes follow]\n{}",
                truncated,
                self.bytes.len(),
                String::from_utf8_lossy(&self.bytes),
            )
        }
    }
}

type SharedTail = Arc<Mutex<Tail>>;

async fn drain<R: tokio::io::AsyncRead + Unpin>(mut reader: R, tail: SharedTail) -> Result<()> {
    let mut chunk = [0u8; 8192];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            return Ok(());
        }
        tail.lock()
            .unwrap_or_else(|error| error.into_inner())
            .append(&chunk[..count]);
    }
}

pub(crate) struct OwnedProcess {
    child: Option<ManagedChild>,
    pub pid: u32,
    pub expect_running: bool,
    pub last_status: Option<ExitStatus>,
    pub last_report: Option<ProcessExit>,
    cleanup_errors: Vec<String>,
    readers: Vec<JoinHandle<Result<()>>>,
    stdout: SharedTail,
    stderr: SharedTail,
    log_prefix: PathBuf,
    #[cfg(unix)]
    tmux_socket: PathBuf,
    #[cfg(target_os = "linux")]
    tmux_tracked_panes: Vec<descendants::Descendants>,
}

impl OwnedProcess {
    pub fn spawn(spec: &ProcessSpec, paths: &CasePaths, epoch: u64) -> Result<Self> {
        spec.validate()?;
        let mut child = agena_process::spawn(spec.command(paths))
            .with_context(|| format!("spawning Agena binary {}", spec.binary.display()))?;
        let pid = child
            .id()
            .context("spawned Agena child has no process id")?;
        let stdout = Arc::new(Mutex::new(Tail::default()));
        let stderr = Arc::new(Mutex::new(Tail::default()));
        let out = child
            .stdout()
            .take()
            .context("Agena stdout pipe was not created")?;
        let err = child
            .stderr()
            .take()
            .context("Agena stderr pipe was not created")?;
        let readers = vec![
            tokio::spawn(drain(out, stdout.clone())),
            tokio::spawn(drain(err, stderr.clone())),
        ];
        Ok(Self {
            child: Some(child),
            pid,
            expect_running: false,
            last_status: None,
            last_report: None,
            readers,
            cleanup_errors: Vec::new(),
            stdout,
            stderr,
            log_prefix: paths.diagnostics.join(format!("server-{epoch}")),
            #[cfg(unix)]
            tmux_socket: paths
                .root
                .join("tmux")
                .join(format!("tmux-{}", unsafe { libc::geteuid() }))
                .join("default"),
            #[cfg(target_os = "linux")]
            tmux_tracked_panes: Vec::new(),
        })
    }

    pub fn poll_exit(&mut self) -> Result<Option<ExitStatus>> {
        if let Some(child) = self.child.as_mut()
            && let Some(status) = child.try_wait()?
        {
            self.last_status = Some(status);
        }
        Ok(self.last_status)
    }

    pub fn is_owned(&self) -> bool {
        self.child.is_some()
    }

    pub fn output(&self) -> (String, String) {
        (
            self.stdout
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .text(),
            self.stderr
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .text(),
        )
    }

    pub async fn write_logs(&self) -> Result<()> {
        let (stdout, stderr) = self.output();
        tokio::fs::write(self.log_prefix.with_extension("stdout.log"), stdout).await?;
        tokio::fs::write(self.log_prefix.with_extension("stderr.log"), stderr).await?;
        Ok(())
    }

    /// A private tmux socket is the only authority for cleaning up PTY
    /// shells. Never connect to or enumerate the user's global tmux service.
    #[cfg(unix)]
    fn tmux_socket_exists(&self) -> Result<bool> {
        use std::os::unix::fs::FileTypeExt;
        let socket = &self.tmux_socket;
        let temp_root = socket
            .parent()
            .and_then(|parent| parent.parent())
            .context("owned tmux socket is missing its private temp root")?;
        for dir in [
            temp_root,
            socket.parent().context("owned tmux socket parent")?,
        ] {
            match std::fs::symlink_metadata(dir) {
                Ok(meta) => ensure_dir(meta, dir)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error).context("inspect private tmux directory"),
            }
        }
        match std::fs::symlink_metadata(socket) {
            Ok(meta) => {
                ensure!(
                    meta.file_type().is_socket(),
                    "private tmux target is not a socket: {}",
                    socket.display()
                );
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).context("inspect private tmux socket"),
        }
    }

    #[cfg(target_os = "linux")]
    async fn inspect_tmux_panes(&self) -> Result<Vec<descendants::Descendants>> {
        if !self.tmux_socket_exists()? {
            return Ok(Vec::new());
        }
        let output = tokio::time::timeout(
            Duration::from_secs(3),
            Command::new("tmux")
                .arg("-S")
                .arg(&self.tmux_socket)
                .args(["list-panes", "-a", "-F", "#{pane_pid}"])
                .env_remove("TMUX")
                .output(),
        )
        .await
        .context("list owned tmux panes timed out")??;
        let mut panes = Vec::new();
        if output.status.success() {
            for line in String::from_utf8(output.stdout)
                .context("non UTF-8 tmux pane list")?
                .lines()
            {
                let pid: u32 = line
                    .parse()
                    .context("invalid pane PID from private tmux server")?;
                panes.push(
                    descendants::Descendants::capture_including_root(pid)
                        .context("capture private tmux pane and children")?,
                );
            }
        }
        Ok(panes)
    }

    /// Stop only the case-owned tmux daemon. Capture pane identity *before*
    /// killing Agena too: a crashed daemon may leave a stale socket and a
    /// shell that ignores HUP, even though no `list-panes` call then works.
    #[cfg(unix)]
    pub async fn cleanup_tmux(&self) -> Result<()> {
        let socket = &self.tmux_socket;
        let available = self.tmux_socket_exists()?;
        #[cfg(target_os = "linux")]
        let mut descendants = self.tmux_tracked_panes.clone();
        #[cfg(target_os = "linux")]
        if available {
            descendants.extend(self.inspect_tmux_panes().await?);
        }
        if available {
            // -S is mandatory: omitting it could destroy the user's sessions.
            let output = tokio::time::timeout(
                Duration::from_secs(3),
                Command::new("tmux")
                    .arg("-S")
                    .arg(socket)
                    .arg("kill-server")
                    .env_remove("TMUX")
                    .output(),
            )
            .await
            .context("private tmux server cleanup timed out")??;
            if !output.status.success() && std::fs::symlink_metadata(socket).is_ok() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("no server running on") {
                    // The daemon already died. Unlink only our case-owned
                    // stale socket after tmux confirmed there is no server.
                    match std::fs::remove_file(socket) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(error).context("unlink stale private tmux socket");
                        }
                    }
                } else {
                    anyhow::bail!(
                        "failed to stop owned tmux server {}: {stderr}",
                        socket.display()
                    );
                }
            }
        }
        #[cfg(target_os = "linux")]
        for pane in descendants {
            tokio::task::spawn_blocking(move || pane.terminate_remaining())
                .await
                .context("join private tmux pane cleanup")??;
        }
        Ok(())
    }

    #[cfg(not(unix))]
    pub async fn cleanup_tmux(&self) -> Result<()> {
        Ok(())
    }

    /// Always kills remaining descendants and reaps the owned leader. A
    /// non-successful Unix exit during ordinary shutdown is a cleanup failure,
    /// including ManagedChild's timeout/SIGKILL fallback.
    pub async fn stop(
        &mut self,
        crash: bool,
        allow_failed_exit: bool,
        grace: Duration,
    ) -> Result<Option<ProcessExit>> {
        let expected_running = self.expect_running;
        self.expect_running = false;
        #[cfg(target_os = "linux")]
        match self.inspect_tmux_panes().await {
            Ok(panes) => self.tmux_tracked_panes.extend(panes),
            Err(error) => self
                .cleanup_errors
                .push(format!("capturing private tmux panes: {error:#}")),
        }
        // Observe the owned tree while the leader still provides its parent
        // chain; independent PTY sessions are not killed by its process group.
        #[cfg(target_os = "linux")]
        let escaped_sessions = match descendants::Descendants::capture(self.pid) {
            Ok(children) => Some(children),
            Err(error) => {
                self.cleanup_errors
                    .push(format!("capturing owned descendants: {error}"));
                None
            }
        };
        let mut report = None;
        // Keep the handle in self across every await. A cancelled restart or
        // crash waiter must leave ownership available to the case's final drain.
        if let Some(child) = self.child.as_mut() {
            let was_exited = match child.try_wait() {
                Ok(status) => status.is_some(),
                Err(error) => {
                    self.cleanup_errors
                        .push(format!("checking owned child before cleanup: {error}"));
                    false
                }
            };
            let stopped = if crash {
                match child.start_kill() {
                    Ok(()) => {
                        match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                            Ok(result) => result.context("waiting for crashed Agena child"),
                            Err(error) => Err(error).context("reaping crashed Agena child"),
                        }
                    }
                    Err(error) => Err(error).context("killing Agena process tree"),
                }
            } else {
                child
                    .terminate(grace)
                    .await
                    .context("terminating Agena process tree")
            };
            let status = match stopped {
                Ok(status) => Some(status),
                Err(error) => {
                    self.cleanup_errors.push(format!("{error:#}"));
                    // A failure to terminate is not an excuse to abandon the
                    // owned child or leave the output readers blocked.
                    if let Err(error) = child.start_kill() {
                        self.cleanup_errors
                            .push(format!("fallback tree kill: {error}"));
                    }
                    match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                        Ok(Ok(status)) => Some(status),
                        Ok(Err(error)) => {
                            self.cleanup_errors
                                .push(format!("fallback child reap: {error}"));
                            None
                        }
                        Err(_) => {
                            self.cleanup_errors
                                .push("fallback child reap timed out".to_owned());
                            None
                        }
                    }
                }
            };
            if let Some(status) = status {
                self.last_status = Some(status);
                if was_exited && expected_running && !crash && !allow_failed_exit {
                    self.cleanup_errors
                        .push(format!("Agena exited before requested shutdown: {status}"));
                }
                if cfg!(unix) && !crash && !allow_failed_exit && !status.success() {
                    self.cleanup_errors
                        .push(format!("Agena did not shut down gracefully: {status}"));
                }
                let kind = if crash {
                    ShutdownKind::Crash
                } else if was_exited {
                    ShutdownKind::AlreadyExited
                } else if cfg!(unix) && status.success() {
                    ShutdownKind::Graceful
                } else {
                    ShutdownKind::Forced
                };
                report = Some(ProcessExit {
                    kind,
                    code: status.code(),
                    status: status.to_string(),
                });
                self.last_report = report.clone();
            }
        }
        // The completed wait/fallback has now reaped the leader or reported an
        // error. ManagedChild's final drop also checks its group/job.
        drop(self.child.take());
        #[cfg(target_os = "linux")]
        if let Some(escaped_sessions) = escaped_sessions {
            match tokio::task::spawn_blocking(move || escaped_sessions.terminate_remaining()).await
            {
                Ok(Ok(count)) if count > 0 && !crash && !allow_failed_exit => {
                    self.cleanup_errors.push(format!(
                        "{count} product descendants outlived graceful shutdown and required forced cleanup"
                    ));
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => self
                    .cleanup_errors
                    .push(format!("terminating owned descendants: {error}")),
                Err(error) => self
                    .cleanup_errors
                    .push(format!("owned descendant cleanup worker: {error}")),
            }
        }
        while !self.readers.is_empty() {
            match tokio::time::timeout(Duration::from_secs(3), &mut self.readers[0]).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => self
                    .cleanup_errors
                    .push(format!("output reader: {error:#}")),
                Ok(Err(error)) => self
                    .cleanup_errors
                    .push(format!("output reader task: {error}")),
                Err(_) => {
                    self.readers[0].abort();
                    let _ = (&mut self.readers[0]).await;
                    self.cleanup_errors
                        .push("output reader did not close after process-tree cleanup".to_owned());
                }
            }
            // Remove only after joining; cancellation of the await above leaves
            // this and every remaining reader owned by the fixture.
            drop(self.readers.remove(0));
        }
        if let Err(error) = self.write_logs().await {
            self.cleanup_errors
                .push(format!("saving bounded process logs: {error:#}"));
        }
        self.last_report = report.clone().or_else(|| self.last_report.clone());
        if self.cleanup_errors.is_empty() {
            Ok(report)
        } else {
            bail!("{}", self.cleanup_errors.join("\n"))
        }
    }
}

#[cfg(unix)]
fn ensure_dir(meta: std::fs::Metadata, path: &std::path::Path) -> Result<()> {
    ensure!(
        meta.file_type().is_dir() && !meta.file_type().is_symlink(),
        "private tmux directory is not a real directory: {}",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[cfg(unix)]
    #[test]
    fn fixture_tmux_socket_is_owned_even_when_environment_override_requests_a_global_server()
    -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let paths = CasePaths::new(temporary.path().to_owned(), &PathBuf::from("workspace"))?;
        let mut overrides = BTreeMap::new();
        overrides.insert(
            OsString::from("TMUX_TMPDIR"),
            Some(OsString::from("/tmp/user-tmux")),
        );
        overrides.insert(
            OsString::from("TMUX"),
            Some(OsString::from("global-session")),
        );
        let environment = isolated_environment(&paths, None, &overrides);
        assert_eq!(
            environment.get(OsStr::new("TMUX_TMPDIR")),
            Some(&paths.root.join("tmux").into_os_string())
        );
        assert!(!environment.contains_key(OsStr::new("TMUX")));
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_stop_waiter_keeps_output_reader_owned_until_join() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut writer, reader) = tokio::io::duplex(128);
        let stdout = Arc::new(Mutex::new(Tail::default()));
        let mut process = OwnedProcess {
            child: None,
            pid: 0,
            expect_running: false,
            last_status: None,
            last_report: None,
            readers: vec![tokio::spawn(drain(reader, stdout.clone()))],
            cleanup_errors: Vec::new(),
            stdout,
            stderr: Arc::new(Mutex::new(Tail::default())),
            log_prefix: directory.path().join("cancelled-stop"),
            #[cfg(unix)]
            tmux_socket: directory.path().join("tmux/tmux-test/default"),
            #[cfg(target_os = "linux")]
            tmux_tracked_panes: Vec::new(),
        };
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                process.stop(false, false, Duration::from_secs(1)),
            )
            .await
            .is_err()
        );
        assert_eq!(process.readers.len(), 1);
        writer.write_all(b"output after cancelled waiter").await?;
        writer.shutdown().await?;
        process.stop(false, false, Duration::from_secs(1)).await?;
        assert!(process.readers.is_empty());
        assert!(process.output().0.contains("output after cancelled waiter"));
        Ok(())
    }

    #[tokio::test]
    async fn capture_keeps_draining_after_its_memory_limit() -> Result<()> {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let tail = Arc::new(Mutex::new(Tail::default()));
        let capture = tokio::spawn(drain(reader, tail.clone()));
        let producer = tokio::spawn(async move {
            writer.write_all(&vec![b'x'; LOG_LIMIT * 4]).await?;
            writer.write_all(b"last diagnostic").await?;
            writer.shutdown().await?;
            Ok::<_, anyhow::Error>(())
        });
        tokio::time::timeout(Duration::from_secs(3), producer).await???;
        tokio::time::timeout(Duration::from_secs(3), capture).await???;
        let tail = tail.lock().unwrap();
        assert_eq!(tail.bytes.len(), LOG_LIMIT);
        assert!(tail.bytes.ends_with(b"last diagnostic"));
        assert!(tail.total_bytes > LOG_LIMIT as u64);
        Ok(())
    }
}
