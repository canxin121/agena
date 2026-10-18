use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::process::Command;

use crate::server::preview::registry::{PreviewSessionRecord, WorkspacePreviewRegistry};
use crate::{ApiResult, AppError};

static PREVIEW_FILE_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

struct ResolvedLogsPaths {
    stdout: PathBuf,
    stderr: PathBuf,
}

fn is_running_state(state: &str) -> bool {
    matches!(state.trim().to_lowercase().as_str(), "running" | "ready")
}

fn resolve_run_directory(raw: &str) -> ApiResult<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("runDirectory is required"));
    }
    let path = PathBuf::from(trimmed);
    if !path.is_dir() {
        return Err(AppError::bad_request(format!(
            "runDirectory does not point to a directory: {}",
            path.to_string_lossy()
        )));
    }
    Ok(path)
}

fn resolve_logs_paths(raw: &str, run_dir: &Path) -> ApiResult<ResolvedLogsPaths> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("logsPath is required"));
    }

    let path = PathBuf::from(trimmed);
    let resolved = if path.is_absolute() {
        path
    } else {
        run_dir.join(path)
    };

    if resolved.is_dir() {
        return Err(AppError::bad_request(format!(
            "logsPath must point to a file, got directory: {}",
            resolved.to_string_lossy()
        )));
    }

    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AppError::internal(format!(
                "failed to create logs directory {}: {err}",
                parent.to_string_lossy()
            ))
        })?;
    }

    Ok(ResolvedLogsPaths {
        stdout: resolved.clone(),
        stderr: resolved,
    })
}

fn resolve_preview_program(program: &str) -> String {
    if program == "bun" {
        for key in ["AGENA_SERVER_BUN_PATH"] {
            if let Some(value) = std::env::var_os(key) {
                let value = value.to_string_lossy().trim().to_string();
                if !value.is_empty() {
                    return value;
                }
            }
        }
    }

    program.to_string()
}

#[cfg(target_os = "macos")]
fn augment_macos_path(base: &str) -> String {
    use std::collections::HashSet;

    let mut out = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();

    for part in base.split(':') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }

    let mut extras = vec![
        "/opt/homebrew/bin".to_string(),
        "/opt/homebrew/sbin".to_string(),
        "/usr/local/bin".to_string(),
        "/usr/local/sbin".to_string(),
    ];

    if let Some(home) = crate::server::path_utils::home_dir_path() {
        extras.push(home.join(".bun").join("bin").to_string_lossy().into_owned());
        extras.push(
            home.join(".cargo")
                .join("bin")
                .to_string_lossy()
                .into_owned(),
        );
    }

    for extra in extras {
        let trimmed = extra.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }

    out.join(":")
}

#[cfg(not(target_os = "macos"))]
fn augment_macos_path(base: &str) -> String {
    base.to_string()
}

#[derive(Clone)]
pub(crate) struct WorkspacePreviewRuntime {
    registry: Arc<WorkspacePreviewRegistry>,
    running_by_id: Arc<DashMap<String, u32>>,
    operations: Arc<tokio::sync::Mutex<()>>,
}

impl WorkspacePreviewRuntime {
    pub(crate) fn new(registry: Arc<WorkspacePreviewRegistry>) -> Self {
        Self {
            registry,
            running_by_id: Arc::new(DashMap::new()),
            operations: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub(crate) async fn start_by_id(&self, id: &str) -> ApiResult<PreviewSessionRecord> {
        let _operation = self.operations.lock().await;
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }

        if self.running_by_id.contains_key(trimmed) {
            return Err(AppError::bad_request("preview session is already running"));
        }

        let session =
            self.registry.get_by_id(trimmed).await.ok_or_else(|| {
                AppError::not_found(format!("preview session not found: {trimmed}"))
            })?;

        if is_running_state(&session.state) {
            return Err(AppError::bad_request(
                "preview session is already marked running; stop it first",
            ));
        }
        if let Some(pid) = session.pid
            && pid > 0
        {
            return Err(AppError::bad_request(
                "preview session has an active pid; stop it first",
            ));
        }

        let run_directory = session.run_directory.clone();
        let logs_path = session.logs_path.clone();
        let (run_dir, stdout_file, stderr_file) = PREVIEW_FILE_WORKERS
            .run(move || {
                let run_dir = resolve_run_directory(&run_directory)?;
                let logs = resolve_logs_paths(&logs_path, &run_dir)?;

                let stdout_file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&logs.stdout)
                    .map_err(|err| {
                        AppError::internal(format!(
                            "failed to open stdout logs file {}: {err}",
                            logs.stdout.to_string_lossy()
                        ))
                    })?;
                let stderr_file = if logs.stdout == logs.stderr {
                    stdout_file.try_clone().map_err(|err| {
                        AppError::internal_error_with_context(
                            "clone the preview logs file handle",
                            &err,
                        )
                    })?
                } else {
                    OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(&logs.stderr)
                        .map_err(|err| {
                            AppError::internal(format!(
                                "failed to open stderr logs file {}: {err}",
                                logs.stderr.to_string_lossy()
                            ))
                        })?
                };

                Ok::<_, AppError>((run_dir, stdout_file, stderr_file))
            })
            .await
            .map_err(|error| AppError::internal_error(&error))??;

        let program = resolve_preview_program(&session.command);
        let mut cmd = Command::new(&program);
        cmd.args(&session.args)
            .current_dir(&run_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .kill_on_drop(true);
        // Dev commands often spawn another server process (npm, Vite, Python
        // reloaders). An owned process group lets Stop terminate the whole app.
        #[cfg(unix)]
        cmd.process_group(0);

        // See plugin_runtime for why this helps on macOS GUI launches.
        if cfg!(target_os = "macos") && !Path::new(&program).is_absolute() {
            let base_path = std::env::var("PATH").unwrap_or_default();
            let augmented = augment_macos_path(&base_path);
            if augmented != base_path && !augmented.is_empty() {
                cmd.env("PATH", augmented);
            }
        }

        let mut child = cmd.spawn().map_err(|err| {
            AppError::bad_gateway(format!(
                "failed to start preview command '{}' (cwd={}): {err}",
                session.command,
                run_dir.to_string_lossy()
            ))
        })?;

        let pid = child.id().ok_or_else(|| {
            AppError::internal("preview process started but pid is unavailable".to_string())
        })?;

        let running_map = self.running_by_id.clone();

        // Track first to ensure stop() can immediately see it.
        running_map.insert(trimmed.to_string(), pid);

        let updated = match self.registry.mark_running_by_id(trimmed, pid).await {
            Ok(updated) => updated,
            Err(error) => {
                if let Err(cleanup) = kill_pid(pid, true).await {
                    tracing::error!(session_id = trimmed, pid, %cleanup, "failed to clean up preview after persistence error");
                } else {
                    running_map.remove(trimmed);
                }
                return Err(error);
            }
        };

        let registry = self.registry.clone();
        let operations = self.operations.clone();
        let session_id = trimmed.to_string();
        tokio::spawn(async move {
            let status = child.wait().await;
            // A prior process exiting must not erase a newer start's pid/state.
            let _operation = operations.lock().await;

            // Only apply exit handling if this process is still the current one.
            let should_apply = match running_map.get(&session_id) {
                Some(current) => *current == pid,
                None => false,
            };

            if should_apply {
                // A launcher can exit while its server child is still alive.
                #[cfg(unix)]
                if let Err(error) = force_kill_unix_processes(pid, true).await {
                    tracing::error!(session_id, pid, %error, "failed to stop exited preview's children");
                    return;
                }
                running_map.remove(&session_id);
            }

            let exit_detail = match status {
                Ok(status) if status.success() => None,
                Ok(status) => Some(format!(
                    "preview process exited with status {:?}",
                    status.code()
                )),
                Err(err) => Some(format!("preview process wait failed: {err}")),
            };

            if should_apply
                && let Err(error) = registry.mark_stopped_by_id(&session_id, exit_detail).await
            {
                tracing::error!(
                    session_id,
                    pid,
                    diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                    "failed to persist preview process exit state"
                );
            }
        });

        Ok(updated)
    }

    pub(crate) async fn stop_by_id(&self, id: &str) -> ApiResult<PreviewSessionRecord> {
        let _operation = self.operations.lock().await;
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }

        let owned_pid = self.running_by_id.get(trimmed).map(|entry| *entry);
        if let Some(pid) = owned_pid {
            kill_pid(pid, true).await?;
            self.running_by_id.remove(trimmed);
            return self.registry.mark_stopped_by_id(trimmed, None).await;
        }

        let session =
            self.registry.get_by_id(trimmed).await.ok_or_else(|| {
                AppError::not_found(format!("preview session not found: {trimmed}"))
            })?;

        if let Some(pid) = session.pid
            && pid > 0
        {
            kill_pid(pid, false).await?;
        }

        self.registry.mark_stopped_by_id(trimmed, None).await
    }
}

async fn kill_pid(pid: u32, owned_group: bool) -> ApiResult<()> {
    if pid == 0 {
        return Ok(());
    }

    #[cfg(unix)]
    {
        if let Err(error) = signal_unix_process(pid, owned_group, libc::SIGTERM, "SIGTERM") {
            if !unix_target_has_live_processes(pid, owned_group).await? {
                return Ok(());
            }
            return Err(error);
        }
        tokio::time::sleep(Duration::from_millis(650)).await;
        return force_kill_unix_processes(pid, owned_group).await;
    }

    #[cfg(windows)]
    {
        let output = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|error| {
                AppError::internal(format!(
                    "failed to execute taskkill for preview process {pid}: {}",
                    agena_failure::diagnostic::format_error_chain(&error)
                ))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::internal(format!(
                "taskkill failed for preview process {pid} with status {}: {}",
                output.status,
                stderr.trim()
            )));
        }
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(unix)]
async fn force_kill_unix_processes(pid: u32, owned_group: bool) -> ApiResult<()> {
    tokio::time::timeout(Duration::from_secs(2), async {
        if let Err(error) = signal_unix_process(pid, owned_group, libc::SIGKILL, "SIGKILL") {
            // Darwin can report EPERM for an all-zombie group. Accept an already
            // dead target only after checking its members, not just the errno.
            if !unix_target_has_live_processes(pid, owned_group).await? {
                return Ok(());
            }
            return Err(error);
        }
        loop {
            if !unix_target_has_live_processes(pid, owned_group).await? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| {
        AppError::internal(format!(
            "preview process {pid} was sent SIGKILL but termination was not confirmed within two seconds"
        ))
    })?
}

#[cfg(unix)]
fn checked_unix_pid(pid: u32) -> ApiResult<libc::pid_t> {
    libc::pid_t::try_from(pid)
        .ok()
        .filter(|pid| *pid > 1)
        .ok_or_else(|| AppError::internal(format!("invalid preview process id: {pid}")))
}

#[cfg(unix)]
async fn unix_target_has_live_processes(pid: u32, owned_group: bool) -> ApiResult<bool> {
    let pid = checked_unix_pid(pid)?;
    tokio::time::timeout(
        Duration::from_secs(2),
        PREVIEW_FILE_WORKERS.run(move || inspect_unix_processes(pid, owned_group)),
    )
    .await
    .map_err(|_| AppError::internal("preview process inspection exceeded two seconds"))?
    .map_err(|error| AppError::internal_error(&error))?
    .map_err(|error| {
        AppError::internal_error_with_context("inspect preview process termination", &error)
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn inspect_unix_processes(pid: libc::pid_t, owned_group: bool) -> std::io::Result<bool> {
    let candidates = if owned_group {
        preview_process_ids()?
    } else {
        vec![pid]
    };
    for candidate in candidates {
        // SAFETY: getpgid only queries metadata for a positive process id.
        let group = unsafe { libc::getpgid(candidate) };
        if group == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            return Err(error);
        }
        if owned_group && group != pid {
            continue;
        }
        if preview_process_is_live(candidate, owned_group.then_some(pid))? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
fn preview_process_ids() -> std::io::Result<Vec<libc::pid_t>> {
    let mut ids = vec![0; 4096];
    loop {
        // SAFETY: ids has exactly the initialized capacity advertised to libproc.
        let count = unsafe {
            libc::proc_listallpids(
                ids.as_mut_ptr().cast(),
                std::mem::size_of_val(ids.as_slice()) as libc::c_int,
            )
        };
        if count <= 0 {
            return Err(std::io::Error::last_os_error());
        }
        if (count as usize) < ids.len() {
            ids.truncate(count as usize);
            ids.retain(|pid| *pid > 1);
            return Ok(ids);
        }
        if ids.len() >= 262_144 {
            return Err(std::io::Error::other(
                "process table exceeds preview cleanup bound",
            ));
        }
        ids.resize(ids.len() * 2, 0);
    }
}

#[cfg(target_os = "macos")]
fn preview_process_is_live(pid: libc::pid_t, group: Option<libc::pid_t>) -> std::io::Result<bool> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // SAFETY: info provides the full-size destination advertised to libproc.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as libc::c_int,
        )
    };
    if read != size as libc::c_int {
        let error = std::io::Error::last_os_error();
        // A member may have exited or left the group during metadata lookup.
        // SAFETY: getpgid only queries metadata for a positive process id.
        let current_group = unsafe { libc::getpgid(pid) };
        if current_group == -1 {
            let lookup_error = std::io::Error::last_os_error();
            if lookup_error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(false);
            }
            return Err(lookup_error);
        }
        if group.is_some_and(|group| current_group != group) {
            return Ok(false);
        }
        return Err(error);
    }
    // SAFETY: the full-size successful libproc read initialized info.
    let info = unsafe { info.assume_init() };
    Ok(group.is_none_or(|group| info.pbi_pgid == group as u32) && info.pbi_status != libc::SZOMB)
}

#[cfg(target_os = "linux")]
fn preview_process_ids() -> std::io::Result<Vec<libc::pid_t>> {
    let mut ids = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str()
            && let Ok(pid) = name.parse::<libc::pid_t>()
            && pid > 1
        {
            ids.push(pid);
            if ids.len() > 262_144 {
                return Err(std::io::Error::other(
                    "process table exceeds preview cleanup bound",
                ));
            }
        }
    }
    Ok(ids)
}

#[cfg(target_os = "linux")]
fn preview_process_is_live(pid: libc::pid_t, group: Option<libc::pid_t>) -> std::io::Result<bool> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    // The comm field may contain spaces and parentheses. Its last ')' precedes
    // state, ppid and pgrp, so splitting the complete line on spaces is invalid.
    let (_, fields) = stat
        .rsplit_once(')')
        .ok_or_else(|| std::io::Error::other("invalid preview process stat"))?;
    let mut fields = fields.split_whitespace();
    let state = fields.next();
    let _parent = fields.next();
    let actual_group = fields
        .next()
        .and_then(|group| group.parse::<libc::pid_t>().ok())
        .ok_or_else(|| std::io::Error::other("invalid preview process group stat"))?;
    Ok(group.is_none_or(|group| actual_group == group) && !matches!(state, Some("Z" | "X" | "x")))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn inspect_unix_processes(pid: libc::pid_t, owned_group: bool) -> std::io::Result<bool> {
    let target = if owned_group { -pid } else { pid };
    // SAFETY: signal 0 only queries existence for a validated process/group id.
    if unsafe { libc::kill(target, 0) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(false);
    }
    Err(error)
}

#[cfg(unix)]
fn signal_unix_process(
    pid: u32,
    owned_group: bool,
    signal: i32,
    signal_name: &str,
) -> ApiResult<()> {
    // SAFETY: libc::kill does not dereference pointers; pid and signal are
    // validated scalar values supplied by this process registry.
    let checked_pid = checked_unix_pid(pid)?;
    let target = if owned_group {
        -checked_pid
    } else {
        checked_pid
    };
    if unsafe { libc::kill(target, signal) } == 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        // The process exited between registry lookup and signal delivery.
        return Ok(());
    }

    Err(AppError::internal(format!(
        "failed to send {signal_name} to preview process {pid}: {}",
        agena_failure::diagnostic::format_error_chain(&error)
    )))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::server::{persistence::db::ServerStateDb, preview::PreviewSessionCreateRequest};

    #[tokio::test]
    async fn stop_terminates_server_children_even_when_they_ignore_sigterm() {
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(
            ServerStateDb::open_at_path(temp.path().join("state.sqlite"))
                .await
                .unwrap(),
        );
        let registry = Arc::new(WorkspacePreviewRegistry::new(db));
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let child = format!(
            "import signal,socket,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);s=socket.socket();s.bind(('127.0.0.1',{}));s.listen();time.sleep(60)",
            address.port()
        );
        let launcher = format!(
            "import subprocess,sys,time;subprocess.Popen([sys.executable,'-c',{}]);time.sleep(60)",
            serde_json::to_string(&child).unwrap()
        );
        registry
            .create_server_session(PreviewSessionCreateRequest {
                id: "children".into(),
                directory: temp.path().to_string_lossy().into_owned(),
                run_directory: temp.path().to_string_lossy().into_owned(),
                agena_session_id: None,
                command: "python3".into(),
                args: vec!["-c".into(), launcher],
                logs_path: String::new(),
                target_url: url::Url::parse(&format!("http://{address}/")).unwrap(),
            })
            .await
            .unwrap();
        runtime.start_by_id("children").await.unwrap();
        let ready = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if tokio::net::TcpStream::connect(address).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        runtime.stop_by_id("children").await.unwrap();
        assert!(
            ready.is_ok(),
            "the child server must have started before Stop"
        );
        assert!(
            tokio::net::TcpStream::connect(address).await.is_err(),
            "Stop must close a child server that ignores SIGTERM"
        );
        assert!(registry.get_by_id("children").await.unwrap().pid.is_none());
    }
}
