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
                running_map.remove(trimmed);
                if let Err(cleanup) = kill_pid(pid, true).await {
                    tracing::error!(session_id = trimmed, pid, %cleanup, "failed to clean up preview after persistence error");
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
                running_map.remove(&session_id);
                // A launcher can exit while its server child is still alive.
                #[cfg(unix)]
                if let Err(error) = signal_unix_process(pid, true, libc::SIGKILL, "SIGKILL") {
                    tracing::error!(session_id, pid, %error, "failed to stop exited preview's children");
                }
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
        signal_unix_process(pid, owned_group, libc::SIGTERM, "SIGTERM")?;
        tokio::time::sleep(Duration::from_millis(650)).await;
        // ESRCH after the grace period means SIGTERM already did its job.
        signal_unix_process(pid, owned_group, libc::SIGKILL, "SIGKILL")?;
        return Ok(());
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
fn signal_unix_process(
    pid: u32,
    owned_group: bool,
    signal: i32,
    signal_name: &str,
) -> ApiResult<()> {
    // SAFETY: libc::kill does not dereference pointers; pid and signal are
    // validated scalar values supplied by this process registry.
    let target = if owned_group {
        -(pid as i32)
    } else {
        pid as i32
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
