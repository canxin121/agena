use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use dashmap::DashMap;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::server::preview::registry::{
    PreviewSessionRecord, PreviewSessionUpdatePatch, WorkspacePreviewRegistry,
    preview_target_from_record,
};
use crate::{ApiResult, AppError};

static PREVIEW_FILE_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

struct ResolvedLogsPaths {
    stdout: PathBuf,
}

fn is_running_state(state: &str) -> bool {
    matches!(state.trim().to_lowercase().as_str(), "running" | "ready")
}

/// Refuse an already occupied target before launching a managed preview. Keep
/// the reservation through file preparation and release it immediately before
/// spawn. This prevents ordinary accidental attachment, but is not proof that
/// the new listener belongs to the child: that final bind remains the app's job.
async fn reserve_preview_target(
    session: &PreviewSessionRecord,
) -> ApiResult<Vec<tokio::net::TcpListener>> {
    let target = preview_target_from_record(session)?;
    let port = target
        .port_or_known_default()
        .filter(|port| *port != 0)
        .ok_or_else(|| AppError::bad_request("preview target must have a nonzero port"))?;
    let localhost = target.host_str() == Some("localhost");
    let addresses: std::collections::BTreeSet<std::net::SocketAddr> = match target.host() {
        Some(url::Host::Ipv4(address)) => [std::net::SocketAddr::from((address, port))]
            .into_iter()
            .collect(),
        Some(url::Host::Ipv6(address)) => [std::net::SocketAddr::from((address, port))]
            .into_iter()
            .collect(),
        Some(url::Host::Domain("localhost")) => tokio::net::lookup_host(("localhost", port))
            .await
            .map_err(|error| {
                AppError::bad_gateway(format!("resolve localhost preview target: {error}"))
            })?
            .collect(),
        _ => return Err(AppError::bad_request("preview target must be loopback")),
    };
    let mut reservations = Vec::new();
    for address in addresses {
        if !address.ip().is_loopback() {
            return Err(AppError::bad_request(
                "localhost preview target resolved outside loopback",
            ));
        }
        match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => reservations.push(listener),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                return Err(AppError::conflict(format!(
                    "preview target address {address} is already in use; stop the existing server or choose another port"
                )));
            }
            // Some hosts resolve ::1 even when the IPv6 interface is disabled.
            Err(error) if localhost && error.kind() == std::io::ErrorKind::AddrNotAvailable => {}
            Err(error) => {
                return Err(AppError::bad_gateway(format!(
                    "failed to reserve preview target address {address}: {error}"
                )));
            }
        }
    }
    if reservations.is_empty() {
        return Err(AppError::bad_gateway(
            "no usable loopback address for preview target",
        ));
    }
    Ok(reservations)
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

pub(super) fn resolve_logs_path(raw: &str, run_dir: &Path) -> ApiResult<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("logsPath is required"));
    }

    let path = PathBuf::from(trimmed);
    Ok(if path.is_absolute() {
        path
    } else {
        run_dir.join(path)
    })
}

fn resolve_logs_paths(raw: &str, run_dir: &Path) -> ApiResult<ResolvedLogsPaths> {
    let resolved = resolve_logs_path(raw, run_dir)?;

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

    Ok(ResolvedLogsPaths { stdout: resolved })
}

fn open_logs_file(path: &Path) -> ApiResult<std::fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO must not pin a blocking worker waiting for a reader. Validate
        // the opened handle too, so a concurrent path replacement cannot turn
        // the log destination into a pipe or device after a metadata precheck.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|err| {
        AppError::internal(format!(
            "failed to open stdout logs file {}: {err}",
            path.to_string_lossy()
        ))
    })?;
    if !file
        .metadata()
        .map_err(|err| AppError::internal_error(&err))?
        .is_file()
    {
        return Err(AppError::bad_request(
            "logsPath must point to a regular file",
        ));
    }
    file.set_len(0)
        .map_err(|err| AppError::internal_error(&err))?;
    Ok(file)
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

/// The live handle, rather than a persisted PID, is the authority to signal a
/// process. ManagedChild owns the complete Unix session / Windows Job Object.
struct OwnedPreview {
    child: Mutex<agena_process::ManagedChild>,
    started_at: i64,
    stop_requested: AtomicBool,
    exited: AtomicBool,
}

impl OwnedPreview {
    fn is_running(&self) -> bool {
        !self.stop_requested.load(Ordering::Acquire) && !self.exited.load(Ordering::Acquire)
    }
}

/// Observe one exact launch without keeping its process alive. App readiness
/// must not silently switch to a replacement started through another endpoint.
pub(super) struct PreviewRun {
    process: Weak<OwnedPreview>,
}

impl PreviewRun {
    pub(super) fn is_running(&self) -> bool {
        self.process
            .upgrade()
            .is_some_and(|process| process.is_running())
    }
}

/// Cancelling an HTTP start during persistence or app readiness must not leave
/// an unobserved process behind. The monitor completes cleanup independently.
pub(super) struct PreviewStartGuard {
    process: Arc<OwnedPreview>,
    committed: bool,
}

impl PreviewStartGuard {
    pub(super) fn commit(mut self) {
        self.committed = true;
    }

    pub(super) fn is_running(&self) -> bool {
        self.process.is_running()
    }

    pub(super) fn run(&self) -> PreviewRun {
        PreviewRun {
            process: Arc::downgrade(&self.process),
        }
    }
}

impl Drop for PreviewStartGuard {
    fn drop(&mut self) {
        if !self.committed {
            self.process.stop_requested.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone)]
pub(crate) struct WorkspacePreviewRuntime {
    registry: Arc<WorkspacePreviewRegistry>,
    running_by_id: Arc<DashMap<String, Arc<OwnedPreview>>>,
    operations: Arc<DashMap<String, Weak<Mutex<()>>>>,
    /// Held only for the synchronous spawn/handle installation or shutdown
    /// admission step. Slow files and persistence cannot delay requesting Stop.
    closed: Arc<std::sync::Mutex<bool>>,
}

impl WorkspacePreviewRuntime {
    pub(crate) fn new(registry: Arc<WorkspacePreviewRegistry>) -> Self {
        Self {
            registry,
            running_by_id: Arc::new(DashMap::new()),
            operations: Arc::new(DashMap::new()),
            closed: Arc::new(std::sync::Mutex::new(false)),
        }
    }

    fn operation(&self, id: &str) -> Arc<Mutex<()>> {
        let mut slot = self.operations.entry(id.to_owned()).or_default();
        if let Some(operation) = slot.upgrade() {
            return operation;
        }
        let operation = Arc::new(Mutex::new(()));
        *slot = Arc::downgrade(&operation);
        operation
    }

    pub(crate) fn is_owned_running(&self, id: &str) -> bool {
        self.running_by_id
            .get(id)
            .is_some_and(|process| process.is_running())
    }

    /// An unlaunched record can represent a user-attached dev server. Once a
    /// managed run has existed, Stop/restart reconciliation must not turn that
    /// record into an attachment to an unrelated process reusing its old port.
    pub(crate) fn can_proxy_record(&self, record: &PreviewSessionRecord) -> bool {
        match self.running_by_id.get(&record.id) {
            Some(process) => process.is_running(),
            None => {
                record.pid.is_none()
                    && record.started_at.is_none()
                    && !is_running_state(&record.state)
            }
        }
    }

    /// Read configuration and capture the running generation under the same
    /// gate used by Start/Stop, including their persistence writes.
    pub(super) async fn owned_session(
        &self,
        id: &str,
    ) -> ApiResult<Option<(PreviewSessionRecord, PreviewRun)>> {
        let operation = self.operation(id);
        let _operation = operation.lock().await;
        let process = self.running_by_id.get(id).map(|entry| entry.clone());
        let Some(process) = process.filter(|process| process.is_running()) else {
            return Ok(None);
        };
        let session = self
            .registry
            .get_by_id(id)
            .await
            .ok_or_else(|| AppError::not_found(format!("preview session not found: {id}")))?;
        Ok(Some((
            session,
            PreviewRun {
                process: Arc::downgrade(&process),
            },
        )))
    }

    /// Mark every service before an app Stop performs its first asynchronous
    /// cleanup, so dropping that request cannot leave later services running.
    pub(super) fn request_stop_by_id(&self, id: &str) {
        if let Some(process) = self.running_by_id.get(id) {
            process.stop_requested.store(true, Ordering::Release);
        }
    }

    /// Reconcile metadata before serving requests after a restart. A persisted
    /// PID may already belong to another process; it is never a kill capability.
    pub(crate) async fn reconcile_stale_sessions(&self) -> ApiResult<()> {
        for session in self.registry.list_all().await?.sessions {
            let operation = self.operation(&session.id);
            let _operation = operation.lock().await;
            if !self.running_by_id.contains_key(&session.id)
                && (session.pid.is_some() || is_running_state(&session.state))
            {
                self.registry
                    .mark_owned_stopped_by_id(
                        &session.id,
                        Some(lost_ownership_message()),
                        session.started_at.unwrap_or(session.updated_at),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn start_by_id(&self, id: &str) -> ApiResult<PreviewSessionRecord> {
        let (session, guard) = self.start_guarded_by_id(id).await?;
        guard.commit();
        Ok(session)
    }

    /// Keep the exact owned process generation pending until a higher-level
    /// operation (such as multi-service readiness) has completed. Dropping the
    /// guard requests cleanup without signalling any later replacement run.
    pub(super) async fn start_guarded_by_id(
        &self,
        id: &str,
    ) -> ApiResult<(PreviewSessionRecord, PreviewStartGuard)> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }
        if *self.closed.lock().unwrap() {
            return Err(AppError::conflict("preview runtime is shutting down"));
        }
        let operation = self.operation(trimmed);
        let _operation = operation.lock().await;

        if self.running_by_id.contains_key(trimmed) {
            return Err(AppError::bad_request("preview session is already running"));
        }

        let mut session =
            self.registry.get_by_id(trimmed).await.ok_or_else(|| {
                AppError::not_found(format!("preview session not found: {trimmed}"))
            })?;
        if is_running_state(&session.state) || session.pid.is_some() {
            session = self
                .registry
                .mark_owned_stopped_by_id(
                    trimmed,
                    Some(lost_ownership_message()),
                    session.started_at.unwrap_or(session.updated_at),
                )
                .await?;
        }

        let reservations = reserve_preview_target(&session).await?;

        let run_directory = session.run_directory.clone();
        let logs_path = session.logs_path.clone();
        let (run_dir, stdout_file, stderr_file) = PREVIEW_FILE_WORKERS
            .run(move || {
                let run_dir = resolve_run_directory(&run_directory)?;
                let logs = resolve_logs_paths(&logs_path, &run_dir)?;
                let stdout_file = open_logs_file(&logs.stdout)?;
                let stderr_file = stdout_file.try_clone().map_err(|err| {
                    AppError::internal_error_with_context(
                        "clone the preview logs file handle",
                        &err,
                    )
                })?;
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
            .stderr(Stdio::from(stderr_file));

        if cfg!(target_os = "macos") && !Path::new(&program).is_absolute() {
            let base_path = std::env::var("PATH").unwrap_or_default();
            let augmented = augment_macos_path(&base_path);
            if augmented != base_path && !augmented.is_empty() {
                cmd.env("PATH", augmented);
            }
        }
        let (pid, pending) = {
            let closed = self.closed.lock().unwrap();
            if *closed {
                return Err(AppError::conflict("preview runtime is shutting down"));
            }
            drop(reservations);
            let child = agena_process::spawn(cmd).map_err(|err| {
                AppError::bad_gateway(format!(
                    "failed to start preview command '{}' (cwd={}): {err}",
                    session.command,
                    run_dir.to_string_lossy()
                ))
            })?;
            let pid = child.id().ok_or_else(|| {
                AppError::internal("preview process started but pid is unavailable")
            })?;
            let process = Arc::new(OwnedPreview {
                child: Mutex::new(child),
                started_at: chrono::Utc::now().timestamp_millis(),
                stop_requested: AtomicBool::new(false),
                exited: AtomicBool::new(false),
            });
            let pending = PreviewStartGuard {
                process: process.clone(),
                committed: false,
            };
            self.running_by_id
                .insert(trimmed.to_owned(), process.clone());
            // No await separates spawn from installing the cancellation guard
            // and monitor. Shutdown sees this handle before admission reopens.
            monitor_process(
                trimmed.to_owned(),
                Arc::downgrade(&process),
                Arc::downgrade(&self.running_by_id),
                operation.clone(),
                self.registry.clone(),
            );
            (pid, pending)
        };
        let updated = self.registry.mark_running_by_id(trimmed, pid).await?;
        if !pending.is_running() {
            return Err(AppError::conflict(
                "preview stopped while its startup was being recorded",
            ));
        }
        Ok((updated, pending))
    }

    pub(crate) async fn stop_by_id(&self, id: &str) -> ApiResult<PreviewSessionRecord> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }
        let operation = self.operation(trimmed);
        let _operation = operation.lock().await;
        let process = self.running_by_id.get(trimmed).map(|entry| entry.clone());
        if let Some(process) = process {
            return self.stop_owned_process(trimmed, &process).await;
        }

        let session =
            self.registry.get_by_id(trimmed).await.ok_or_else(|| {
                AppError::not_found(format!("preview session not found: {trimmed}"))
            })?;
        let stale = session.pid.is_some() || is_running_state(&session.state);
        if stale {
            self.registry
                .mark_owned_stopped_by_id(
                    trimmed,
                    Some(lost_ownership_message()),
                    session.started_at.unwrap_or(session.updated_at),
                )
                .await
        } else {
            self.registry.mark_stopped_by_id(trimmed, None).await
        }
    }

    // Registry commits survive caller cancellation. Keep the corresponding
    // runtime gate alive through the same commit, including Delete's cleanup.
    pub(crate) async fn update_by_id(
        &self,
        id: &str,
        patch: PreviewSessionUpdatePatch,
    ) -> ApiResult<PreviewSessionRecord> {
        let runtime = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move { runtime.update_inner(&id, patch).await })
            .await
            .map_err(|error| AppError::internal(format!("preview update task failed: {error}")))?
    }

    async fn update_inner(
        &self,
        id: &str,
        patch: PreviewSessionUpdatePatch,
    ) -> ApiResult<PreviewSessionRecord> {
        let id = id.trim();
        let operation = self.operation(id);
        let _operation = operation.lock().await;
        if self.running_by_id.contains_key(id) {
            return Err(AppError::conflict(
                "stop the preview and wait for cleanup before changing its configuration",
            ));
        }
        self.registry.update_by_id(id, patch).await
    }

    pub(crate) async fn rename_by_id(
        &self,
        id: &str,
        new_id: &str,
    ) -> ApiResult<PreviewSessionRecord> {
        let runtime = self.clone();
        let id = id.to_owned();
        let new_id = new_id.to_owned();
        tokio::spawn(async move { runtime.rename_inner(&id, &new_id).await })
            .await
            .map_err(|error| AppError::internal(format!("preview rename task failed: {error}")))?
    }

    async fn rename_inner(&self, id: &str, new_id: &str) -> ApiResult<PreviewSessionRecord> {
        let id = id.trim();
        let new_id = new_id.trim();
        // Renaming changes the runtime lookup key. Gate both names in stable
        // order so concurrent starts and opposite renames cannot race/deadlock.
        let (first, second) = if id <= new_id {
            (id, new_id)
        } else {
            (new_id, id)
        };
        let first_operation = self.operation(first);
        let _first = first_operation.lock().await;
        let second_operation = (first != second).then(|| self.operation(second));
        let _second = match &second_operation {
            Some(operation) => Some(operation.lock().await),
            None => None,
        };
        if self.running_by_id.contains_key(id) || self.running_by_id.contains_key(new_id) {
            return Err(AppError::conflict(
                "stop the preview and wait for cleanup before renaming it",
            ));
        }
        self.registry.rename_by_id(id, new_id).await
    }

    pub(crate) async fn delete_by_id(&self, id: &str) -> ApiResult<()> {
        let runtime = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move { runtime.delete_inner(&id).await })
            .await
            .map_err(|error| AppError::internal(format!("preview delete task failed: {error}")))?
    }

    async fn delete_inner(&self, id: &str) -> ApiResult<()> {
        let id = id.trim();
        let operation = self.operation(id);
        let _operation = operation.lock().await;
        let process = self.running_by_id.get(id).map(|entry| entry.clone());
        if let Some(process) = process {
            self.stop_owned_process(id, &process).await?;
        }
        // Keep the gate until the record is gone: releasing it after Stop
        // would permit a queued Start to launch before the subsequent delete.
        self.registry.delete_by_id(id).await
    }

    /// Rollback owns a specific launch, so it must never stop a replacement
    /// started via another API while the app was waiting for readiness.
    pub(super) async fn rollback_start(
        &self,
        id: &str,
        guard: &PreviewStartGuard,
    ) -> ApiResult<()> {
        let operation = self.operation(id);
        let _operation = operation.lock().await;
        if self
            .running_by_id
            .get(id)
            .is_some_and(|process| Arc::ptr_eq(&process, &guard.process))
        {
            self.stop_owned_process(id, &guard.process).await?;
        }
        Ok(())
    }

    /// Called only while holding this session's operation gate.
    async fn stop_owned_process(
        &self,
        id: &str,
        process: &OwnedPreview,
    ) -> ApiResult<PreviewSessionRecord> {
        process.stop_requested.store(true, Ordering::Release);
        {
            let mut child = process.child.lock().await;
            // The monitor can complete cleanup independently while a registry
            // mutation is blocked. Recheck under its same child lock to avoid
            // signalling a reaped group whose numeric ID may already be reused.
            if !process.exited.load(Ordering::Acquire) {
                child
                    .terminate(Duration::from_millis(650))
                    .await
                    .map_err(|error| {
                        AppError::internal_error_with_context(
                            "terminate the owned preview process tree",
                            &error,
                        )
                    })?;
                process.exited.store(true, Ordering::Release);
            }
        }
        let updated = self
            .registry
            .mark_owned_stopped_by_id(id, None, process.started_at)
            .await?;
        self.running_by_id.remove(id);
        Ok(updated)
    }

    /// Stop admission first, then terminate all owned trees concurrently. No
    /// unrelated process is signalled when stale records remain on disk.
    pub(crate) async fn shutdown(&self) -> ApiResult<()> {
        let ids: Vec<_> = {
            let mut closed = self.closed.lock().unwrap();
            *closed = true;
            self.running_by_id
                .iter()
                .map(|entry| {
                    entry.stop_requested.store(true, Ordering::Release);
                    entry.key().clone()
                })
                .collect()
        };
        let results =
            futures_util::future::join_all(ids.iter().map(|id| self.stop_by_id(id))).await;
        let mut first_error = None;
        for result in results {
            if let Err(error) = result {
                tracing::error!(%error, "failed to stop a preview during shutdown");
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn lost_ownership_message() -> String {
    "preview process ownership was lost after a restart; the stored PID was not signalled".into()
}

fn monitor_process(
    id: String,
    process: Weak<OwnedPreview>,
    processes: Weak<DashMap<String, Arc<OwnedPreview>>>,
    operation: Arc<Mutex<()>>,
    registry: Arc<WorkspacePreviewRegistry>,
) {
    tokio::spawn(async move {
        let mut delay = Duration::from_millis(100);
        let mut terminal_detail = None;
        loop {
            tokio::time::sleep(delay).await;
            let (Some(process), Some(processes)) = (process.upgrade(), processes.upgrade()) else {
                return;
            };
            if !processes
                .get(&id)
                .is_some_and(|current| Arc::ptr_eq(&current, &process))
            {
                return;
            }
            let cleanup = {
                let mut child = process.child.lock().await;
                let result = if process.exited.load(Ordering::Acquire) {
                    // Recheck under the same lock as explicit Stop. No waiter
                    // may re-signal a group after successful cleanup/reaping.
                    Ok(())
                } else if process.stop_requested.load(Ordering::Acquire) {
                    child
                        .terminate(Duration::from_millis(650))
                        .await
                        .map(|_| ())
                } else {
                    match child.try_wait() {
                        Ok(None) => continue,
                        Ok(Some(status)) => {
                            process.stop_requested.store(true, Ordering::Release);
                            terminal_detail = (!status.success()).then(|| {
                                format!("preview process exited with status {:?}", status.code())
                            });
                            // A launcher can exit while a descendant is still alive.
                            child.start_kill()
                        }
                        Err(error) => {
                            process.stop_requested.store(true, Ordering::Release);
                            terminal_detail = Some(format!("preview process wait failed: {error}"));
                            child
                                .terminate(Duration::from_millis(650))
                                .await
                                .map(|_| ())
                        }
                    }
                };
                if result.is_ok() {
                    process.exited.store(true, Ordering::Release);
                }
                result
            };
            if let Err(error) = cleanup {
                tracing::error!(session_id = id, %error, "failed to clean up owned preview process tree");
                delay = Duration::from_secs(1);
                continue;
            }
            // Process cleanup is independent of slow registry writes. Only
            // metadata/removal waits for the operation gate, after which this
            // exact generation must still be installed.
            let _operation = operation.lock().await;
            if !processes
                .get(&id)
                .is_some_and(|current| Arc::ptr_eq(&current, &process))
            {
                return;
            }
            match registry
                .mark_owned_stopped_by_id(&id, terminal_detail.clone(), process.started_at)
                .await
            {
                Ok(_) => {
                    processes.remove(&id);
                    return;
                }
                Err(error) => {
                    tracing::error!(session_id = id, %error, "failed to persist preview process exit state");
                    delay = Duration::from_secs(1);
                }
            }
        }
    });
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::server::persistence::db::ServerStateDb;
    use crate::server::preview::registry::PreviewSessionCreateRequest;
    use std::collections::HashMap;

    pub(crate) struct TestStore {
        path: PathBuf,
        values: std::sync::RwLock<HashMap<String, String>>,
        block_running_write: AtomicBool,
        pub(crate) block_stopped_write: AtomicBool,
        corrupt_next_read: AtomicBool,
        block_read: std::sync::Mutex<Option<(String, usize)>>,
        block_write: std::sync::Mutex<Option<String>>,
        pub(crate) blocked: tokio::sync::Notify,
        pub(crate) resume: tokio::sync::Notify,
    }

    impl TestStore {
        pub(crate) fn new(root: &Path) -> Self {
            Self {
                path: root.join("preview-tests.sqlite3"),
                values: Default::default(),
                block_running_write: AtomicBool::new(false),
                block_stopped_write: AtomicBool::new(false),
                corrupt_next_read: AtomicBool::new(false),
                block_read: std::sync::Mutex::new(None),
                block_write: std::sync::Mutex::new(None),
                blocked: tokio::sync::Notify::new(),
                resume: tokio::sync::Notify::new(),
            }
        }

        pub(crate) fn block_read_after(&self, key: String, skip: usize) {
            *self.block_read.lock().unwrap() = Some((key, skip));
        }

        pub(crate) fn block_next_write(&self, key: String) {
            *self.block_write.lock().unwrap() = Some(key);
        }
    }

    #[async_trait::async_trait]
    impl agena_storage::instance_state::InstanceStateRepository for TestStore {
        fn path(&self) -> &Path {
            &self.path
        }

        async fn get_payload(
            &self,
            key: &str,
        ) -> Result<Option<String>, agena_storage::instance_state::InstanceStateError> {
            let block = {
                let mut pending = self.block_read.lock().unwrap();
                match pending.as_mut() {
                    Some((expected, skip)) if expected.as_str() == key => {
                        if *skip == 0 {
                            *pending = None;
                            true
                        } else {
                            *skip -= 1;
                            false
                        }
                    }
                    _ => false,
                }
            };
            if block {
                self.blocked.notify_one();
                self.resume.notified().await;
            }
            if self.corrupt_next_read.swap(false, Ordering::AcqRel) {
                return Ok(Some("invalid JSON injected for persistence retry".into()));
            }
            Ok(self.values.read().unwrap().get(key).cloned())
        }

        async fn set_payload(
            &self,
            key: &str,
            payload: String,
        ) -> Result<(), agena_storage::instance_state::InstanceStateError> {
            let block_key = {
                let mut pending = self.block_write.lock().unwrap();
                if pending.as_deref() == Some(key) {
                    *pending = None;
                    true
                } else {
                    false
                }
            };
            if block_key
                || payload.contains("\"state\":\"running\"")
                    && self.block_running_write.swap(false, Ordering::AcqRel)
                || payload.contains("\"state\":\"stopped\"")
                    && self.block_stopped_write.swap(false, Ordering::AcqRel)
            {
                self.blocked.notify_one();
                self.resume.notified().await;
            }
            self.values.write().unwrap().insert(key.into(), payload);
            Ok(())
        }
    }

    pub(crate) async fn test_registry(
        root: &Path,
    ) -> (Arc<ServerStateDb>, Arc<WorkspacePreviewRegistry>) {
        let db = Arc::new(
            ServerStateDb::from_store(Arc::new(TestStore::new(root)))
                .await
                .unwrap(),
        );
        let registry = Arc::new(WorkspacePreviewRegistry::new(db.clone()));
        (db, registry)
    }

    async fn create_session(
        registry: &WorkspacePreviewRegistry,
        root: &Path,
        id: &str,
        script: &str,
    ) {
        let target = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = target.local_addr().unwrap().port();
        registry
            .create_server_session(PreviewSessionCreateRequest {
                id: id.into(),
                directory: root.to_string_lossy().into_owned(),
                run_directory: root.to_string_lossy().into_owned(),
                agena_session_id: None,
                command: "sh".into(),
                args: vec!["-c".into(), script.into()],
                logs_path: root
                    .join(format!("{id}.log"))
                    .to_string_lossy()
                    .into_owned(),
                target_url: url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
            })
            .await
            .unwrap();
    }

    async fn wait_until_removed(runtime: &WorkspacePreviewRuntime, id: &str) {
        tokio::time::timeout(Duration::from_secs(4), async {
            while runtime.running_by_id.contains_key(id) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("preview process cleanup should finish");
    }

    #[tokio::test]
    async fn restart_reconciliation_clears_stale_metadata_without_signalling_a_pid() {
        let directory = tempfile::tempdir().unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        create_session(&registry, directory.path(), "stale", "exit 0").await;
        registry
            .mark_running_by_id("stale", u32::MAX)
            .await
            .unwrap();
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        runtime.reconcile_stale_sessions().await.unwrap();
        let session = registry.get_by_id("stale").await.unwrap();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.pid, None);
        assert!(session.error.unwrap().contains("not signalled"));
        runtime.reconcile_stale_sessions().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stopping_a_persisted_pid_does_not_kill_an_unrelated_live_process() {
        let directory = tempfile::tempdir().unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        create_session(&registry, directory.path(), "stale", "exit 0").await;
        let mut command = Command::new("sh");
        command.args(["-c", "exec sleep 30"]);
        let mut unrelated = agena_process::spawn(command).unwrap();
        registry
            .mark_running_by_id("stale", unrelated.id().unwrap())
            .await
            .unwrap();
        let runtime = WorkspacePreviewRuntime::new(registry);
        let stopped = runtime.stop_by_id("stale").await.unwrap();
        assert_eq!(stopped.pid, None);
        assert!(
            unrelated.try_wait().unwrap().is_none(),
            "stored PID must not authorize signalling"
        );
        unrelated
            .terminate(Duration::from_millis(50))
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_a_config_update_keeps_start_behind_its_commit() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore::new(directory.path()));
        let db = Arc::new(ServerStateDb::from_store(store.clone()).await.unwrap());
        let registry = Arc::new(WorkspacePreviewRegistry::new(db));
        create_session(&registry, directory.path(), "updated", "exec sleep 30").await;
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        store.block_stopped_write.store(true, Ordering::Release);
        let updating = {
            let runtime = runtime.clone();
            tokio::spawn(async move {
                runtime
                    .update_by_id(
                        "updated",
                        PreviewSessionUpdatePatch {
                            command: Some("sleep".into()),
                            args: Some(vec!["30".into()]),
                            ..Default::default()
                        },
                    )
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        updating.abort();
        assert!(updating.await.unwrap_err().is_cancelled());
        let operation = runtime.operation("updated");
        assert!(
            operation.try_lock().is_err(),
            "cancelling the HTTP caller must not release the mutation gate"
        );
        let starting = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.start_by_id("updated").await })
        };
        tokio::task::yield_now().await;
        assert!(!starting.is_finished());
        store.resume.notify_one();
        let started = starting.await.unwrap().unwrap();
        assert_eq!(started.command, "sleep");
        assert_eq!(started.args, vec!["30".to_string()]);
        runtime.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_fifo_cannot_be_used_as_a_managed_preview_log_file() {
        use std::os::unix::fs::OpenOptionsExt;
        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("preview.fifo");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: c_path is a valid NUL-terminated path; mkfifo retains no pointer.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let reader = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
            .unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        create_session(&registry, directory.path(), "fifo", "exec sleep 30").await;
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        runtime
            .update_by_id(
                "fifo",
                PreviewSessionUpdatePatch {
                    logs_path: Some(fifo.to_string_lossy().into_owned()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), runtime.start_by_id("fifo"))
            .await
            .unwrap();
        assert!(matches!(result, Err(error) if error.to_string().contains("regular file")));
        assert!(!runtime.running_by_id.contains_key("fifo"));
        drop(reader);
        assert!(
            open_logs_file(&fifo).is_err(),
            "opening a FIFO without a reader must fail without blocking"
        );
        runtime.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_occupied_preview_target_never_launches_or_truncates_existing_logs() {
        let directory = tempfile::tempdir().unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        create_session(&registry, directory.path(), "occupied", "exec sleep 30").await;
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        let log_path = directory.path().join("occupied.log");
        tokio::fs::write(&log_path, "previous diagnostic")
            .await
            .unwrap();
        for host in ["127.0.0.1", "localhost"] {
            runtime
                .update_by_id(
                    "occupied",
                    PreviewSessionUpdatePatch {
                        target_url: Some(format!("http://{host}:{port}/")),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            let result = runtime.start_by_id("occupied").await;
            assert!(matches!(result, Err(error) if error.to_string().contains("already in use")));
            assert!(!runtime.running_by_id.contains_key("occupied"));
            assert_eq!(
                tokio::fs::read_to_string(&log_path).await.unwrap(),
                "previous diagnostic"
            );
            assert_eq!(registry.get_by_id("occupied").await.unwrap().pid, None);
        }
        drop(listener);
        runtime.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stopped_managed_preview_never_becomes_an_external_attachment() {
        let directory = tempfile::tempdir().unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        create_session(&registry, directory.path(), "owned", "exec sleep 30").await;
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        let created = registry.get_by_id("owned").await.unwrap();
        assert!(
            runtime.can_proxy_record(&created),
            "a never-launched record can attach an external server"
        );
        let running = runtime.start_by_id("owned").await.unwrap();
        assert!(runtime.can_proxy_record(&running));
        let stopped = runtime.stop_by_id("owned").await.unwrap();
        assert!(stopped.started_at.is_some());
        assert!(!runtime.can_proxy_record(&stopped));
        let restarted_runtime = WorkspacePreviewRuntime::new(registry.clone());
        restarted_runtime.reconcile_stale_sessions().await.unwrap();
        assert!(!restarted_runtime.can_proxy_record(&registry.get_by_id("owned").await.unwrap()));
        runtime.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn owned_stop_restart_and_shutdown_preserve_the_current_process_generation() {
        let directory = tempfile::tempdir().unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        create_session(&registry, directory.path(), "owned", "exec sleep 30").await;
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        runtime.start_by_id("owned").await.unwrap();
        assert!(runtime.is_owned_running("owned"));
        runtime.stop_by_id("owned").await.unwrap();
        let restarted = runtime.start_by_id("owned").await.unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(runtime.is_owned_running("owned"));
        assert_eq!(
            registry.get_by_id("owned").await.unwrap().pid,
            restarted.pid
        );
        runtime.shutdown().await.unwrap();
        assert!(!runtime.is_owned_running("owned"));
        assert_eq!(registry.get_by_id("owned").await.unwrap().pid, None);
        assert!(runtime.start_by_id("owned").await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn natural_process_exit_persists_its_status_and_releases_ownership() {
        let directory = tempfile::tempdir().unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        create_session(&registry, directory.path(), "exits", "exit 7").await;
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        runtime.start_by_id("exits").await.unwrap();
        wait_until_removed(&runtime, "exits").await;
        let session = registry.get_by_id("exits").await.unwrap();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.pid, None);
        assert!(session.error.unwrap().contains('7'));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_exit_diagnostic_survives_a_failed_persistence_attempt() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore::new(directory.path()));
        let db = Arc::new(ServerStateDb::from_store(store.clone()).await.unwrap());
        let registry = Arc::new(WorkspacePreviewRegistry::new(db));
        create_session(&registry, directory.path(), "retry", "sleep 0.2; exit 7").await;
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        runtime.start_by_id("retry").await.unwrap();
        store.corrupt_next_read.store(true, Ordering::Release);
        wait_until_removed(&runtime, "retry").await;
        let session = registry.get_by_id("retry").await.unwrap();
        assert_eq!(session.pid, None);
        assert!(
            session.error.unwrap().contains('7'),
            "a failed persistence attempt must not erase the natural exit diagnostic"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_or_rolling_back_a_start_never_stops_a_later_generation() {
        let directory = tempfile::tempdir().unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        create_session(&registry, directory.path(), "generation", "exec sleep 30").await;
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        let (_, old_start) = runtime.start_guarded_by_id("generation").await.unwrap();
        runtime.stop_by_id("generation").await.unwrap();
        let replacement = runtime.start_by_id("generation").await.unwrap();
        runtime
            .rollback_start("generation", &old_start)
            .await
            .unwrap();
        drop(old_start);
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(runtime.is_owned_running("generation"));
        assert_eq!(
            registry.get_by_id("generation").await.unwrap().pid,
            replacement.pid
        );
        runtime.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn owned_previews_require_stop_before_identity_or_configuration_changes() {
        let directory = tempfile::tempdir().unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        create_session(&registry, directory.path(), "original", "exec sleep 30").await;
        let runtime = WorkspacePreviewRuntime::new(registry.clone());
        let original = runtime.start_by_id("original").await.unwrap();
        let patch = PreviewSessionUpdatePatch {
            command: Some("different-program".into()),
            ..Default::default()
        };
        assert!(
            runtime
                .update_by_id("original", patch.clone())
                .await
                .is_err()
        );
        assert!(runtime.rename_by_id("original", "renamed").await.is_err());
        assert_eq!(registry.get_by_id("original").await.unwrap(), original);
        assert!(registry.get_by_id("renamed").await.is_none());
        assert!(runtime.is_owned_running("original"));
        runtime.stop_by_id("original").await.unwrap();
        assert_eq!(
            runtime
                .update_by_id("original", patch)
                .await
                .unwrap()
                .command,
            "different-program"
        );
        assert_eq!(
            runtime
                .rename_by_id("original", "renamed")
                .await
                .unwrap()
                .id,
            "renamed"
        );
        runtime.delete_by_id("renamed").await.unwrap();
        assert!(registry.get_by_id("renamed").await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn delete_waits_for_an_in_flight_start_and_cleans_up_that_launch() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore::new(directory.path()));
        let db = Arc::new(ServerStateDb::from_store(store.clone()).await.unwrap());
        let registry = Arc::new(WorkspacePreviewRegistry::new(db));
        create_session(&registry, directory.path(), "deleting", "exec sleep 30").await;
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        store.block_running_write.store(true, Ordering::Release);
        let starting = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.start_by_id("deleting").await })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        let process = runtime.running_by_id.get("deleting").unwrap().clone();
        let deleting = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.delete_by_id("deleting").await })
        };
        tokio::task::yield_now().await;
        assert!(!deleting.is_finished());
        store.resume.notify_one();
        assert!(starting.await.unwrap().is_ok());
        deleting.await.unwrap().unwrap();
        assert!(!runtime.running_by_id.contains_key("deleting"));
        assert!(process.child.lock().await.try_wait().unwrap().is_some());
        assert!(registry.get_by_id("deleting").await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn delete_keeps_the_gate_through_stop_and_record_removal() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore::new(directory.path()));
        let db = Arc::new(ServerStateDb::from_store(store.clone()).await.unwrap());
        let registry = Arc::new(WorkspacePreviewRegistry::new(db));
        create_session(&registry, directory.path(), "deleting", "exec sleep 30").await;
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        runtime.start_by_id("deleting").await.unwrap();
        store.block_stopped_write.store(true, Ordering::Release);
        let deleting = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.delete_by_id("deleting").await })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        let starting = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.start_by_id("deleting").await })
        };
        tokio::task::yield_now().await;
        assert!(!starting.is_finished());
        store.resume.notify_one();
        deleting.await.unwrap().unwrap();
        assert!(
            starting.await.unwrap().is_err(),
            "a start queued behind Delete must observe the deleted record"
        );
        assert!(!runtime.running_by_id.contains_key("deleting"));
        assert!(registry.get_by_id("deleting").await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_cleans_processes_even_when_start_persistence_and_shutdown_wait_are_cancelled()
    {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore::new(directory.path()));
        let db = Arc::new(ServerStateDb::from_store(store.clone()).await.unwrap());
        let registry = Arc::new(WorkspacePreviewRegistry::new(db));
        create_session(&registry, directory.path(), "blocked", "exec sleep 30").await;
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        store.block_running_write.store(true, Ordering::Release);
        let starting = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.start_by_id("blocked").await })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        let process = runtime.running_by_id.get("blocked").unwrap().clone();
        let shutting_down = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.shutdown().await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while !process.stop_requested.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown must close admission before waiting for startup persistence");
        shutting_down.abort();
        assert!(shutting_down.await.unwrap_err().is_cancelled());
        assert!(
            tokio::time::timeout(Duration::from_secs(1), runtime.start_by_id("blocked"))
                .await
                .unwrap()
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(3), async {
            while !process.exited.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("process cleanup must not wait for the blocked registry write");
        assert!(process.child.lock().await.try_wait().unwrap().is_some());
        assert!(!starting.is_finished());
        store.resume.notify_one();
        assert!(starting.await.unwrap().is_err());
        wait_until_removed(&runtime, "blocked").await;
        assert_eq!(registry.get_by_id("blocked").await.unwrap().pid, None);
        runtime.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_start_during_persistence_cleans_up_the_owned_tree() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore::new(directory.path()));
        let db = Arc::new(ServerStateDb::from_store(store.clone()).await.unwrap());
        let registry = Arc::new(WorkspacePreviewRegistry::new(db));
        create_session(&registry, directory.path(), "cancelled", "exec sleep 30").await;
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        store.block_running_write.store(true, Ordering::Release);
        let starting = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.start_by_id("cancelled").await })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        assert!(runtime.running_by_id.contains_key("cancelled"));
        let process = runtime.running_by_id.get("cancelled").unwrap().clone();
        starting.abort();
        assert!(starting.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(3), async {
            while !process.exited.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelling Start must clean the process before a blocked DB commit completes");
        store.resume.notify_one();
        wait_until_removed(&runtime, "cancelled").await;
        let cancelled = registry.get_by_id("cancelled").await.unwrap();
        assert_eq!(cancelled.pid, None);
        assert!(cancelled.started_at.is_some());
        assert!(!runtime.can_proxy_record(&cancelled));
    }
}
