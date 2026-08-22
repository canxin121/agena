use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex, Once};
use std::thread;
use std::time::{Duration, Instant};

use process_control::{ChildExt as _, Control as _};
use tempfile::TempDir;

use crate::CrawlError;

#[derive(Debug, Clone)]
/// Options for launching a local browser.
pub struct LocalBrowserOptions {
    pub executable_path: Option<PathBuf>,
    pub startup_timeout: Duration,
    /// How long the managed browser may stay idle before it is shut down
    /// automatically. `None` disables idle auto-close.
    pub idle_timeout: Option<Duration>,
}

impl Default for LocalBrowserOptions {
    fn default() -> Self {
        Self {
            executable_path: None,
            startup_timeout: Duration::from_secs(10),
            idle_timeout: None,
        }
    }
}

struct ManagedBrowser {
    child: Child,
    endpoint: String,
    _profile_dir: TempDir,
}

impl ManagedBrowser {
    fn spawn(options: &LocalBrowserOptions) -> Result<Self, CrawlError> {
        let executable = find_browser_executable(options.executable_path.as_deref())?;
        let profile_dir = tempfile::Builder::new()
            .prefix("agena-web-browser-")
            .tempdir()?;
        let mut command = Command::new(&executable);
        command
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg("--no-sandbox")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--remote-debugging-address=127.0.0.1")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile_dir.path().display()))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Put the browser in its own process group so shutdown can kill the
        // whole tree (helpers included), not just the main process.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|err| {
            CrawlError::InvalidInput(format!(
                "failed to launch local browser '{}': {err}",
                executable.display()
            ))
        })?;

        let endpoint = match wait_for_devtools_endpoint(
            &mut child,
            profile_dir.path(),
            options.startup_timeout,
        ) {
            Ok(endpoint) => endpoint,
            Err(err) => {
                return match shutdown_browser_process(&mut child) {
                    Ok(()) => Err(err),
                    Err(cleanup_error) => Err(CrawlError::InvalidInput(format!(
                        "{}; additionally, {}",
                        agena_failure::diagnostic::format_error_chain(&err),
                        agena_failure::diagnostic::format_error_chain_with_context(
                            "failed to clean up the local browser after startup failed",
                            &cleanup_error,
                        )
                    ))),
                };
            }
        };

        tracing::debug!(
            target: "agena::web",
            executable = %executable.display(),
            endpoint = %endpoint,
            "started managed local browser for crawl rendering"
        );

        Ok(Self {
            child,
            endpoint,
            _profile_dir: profile_dir,
        })
    }

    fn is_running(&mut self) -> Result<bool, CrawlError> {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(|error| {
                CrawlError::InvalidInput(
                    agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to inspect the managed local browser process",
                        &error,
                    ),
                )
            })
    }

    /// Kill the browser process tree. `TempDir` removes the profile when this
    /// managed entry is dropped, including startup-error paths.
    fn shutdown(&mut self) -> Result<(), CrawlError> {
        if !self.is_running()? {
            return Ok(());
        }
        shutdown_browser_process(&mut self.child)
    }
}

impl Drop for ManagedBrowser {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            tracing::error!(
                target: "agena::web",
                error = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to shut down the managed local browser while dropping it",
                    &error,
                ),
                "managed local browser cleanup failed"
            );
        }
    }
}

/// Process-lifetime registry for the single managed browser. Rust statics are
/// never dropped at process exit, so the child MUST be shut down explicitly
/// through [`shutdown_local_browser`] (plugin shutdown, idle auto-close, or an
/// explicit management tool); relying on `Drop` alone would leak the browser
/// process after the host exits.
struct LocalBrowserState {
    browser: Option<ManagedBrowser>,
    last_used: Option<Instant>,
    idle_timeout: Option<Duration>,
}

static LOCAL_BROWSER: LazyLock<Mutex<LocalBrowserState>> = LazyLock::new(|| {
    Mutex::new(LocalBrowserState {
        browser: None,
        last_used: None,
        idle_timeout: None,
    })
});

/// Return the DevTools WebSocket endpoint of the managed browser, spawning it
/// lazily on first use. The browser is reused for the process lifetime and is
/// shut down by [`shutdown_local_browser`] or after `options.idle_timeout` of
/// inactivity.
pub fn local_browser_endpoint(options: &LocalBrowserOptions) -> Result<String, CrawlError> {
    let mut state = LOCAL_BROWSER.lock().map_err(|error| {
        CrawlError::InvalidInput(agena_failure::diagnostic::format_error_chain_with_context(
            "local browser registry mutex is poisoned",
            &error,
        ))
    })?;
    if let Some(existing) = state.browser.as_mut()
        && existing.is_running()?
    {
        let endpoint = existing.endpoint.clone();
        state.last_used = Some(Instant::now());
        state.idle_timeout = options.idle_timeout;
        return Ok(endpoint);
    }

    // Drop a dead entry (kills and cleans its profile dir) before respawning.
    state.browser = None;
    let browser = ManagedBrowser::spawn(options)?;
    let endpoint = browser.endpoint.clone();
    state.idle_timeout = options.idle_timeout;
    state.last_used = Some(Instant::now());
    state.browser = Some(browser);
    if options.idle_timeout.is_some() {
        ensure_idle_janitor();
    }
    Ok(endpoint)
}

/// Report whether the managed browser process is currently running. This never
/// starts a browser; management tools use it to inspect state cheaply.
pub fn local_browser_running() -> Result<bool, CrawlError> {
    let mut state = LOCAL_BROWSER.lock().map_err(|error| {
        CrawlError::InvalidInput(agena_failure::diagnostic::format_error_chain_with_context(
            "local browser registry mutex is poisoned",
            &error,
        ))
    })?;
    match state.browser.as_mut() {
        Some(browser) => browser.is_running(),
        None => Ok(false),
    }
}

/// Mark the managed browser as recently used so the idle auto-close timer
/// restarts. No-op when no browser is running.
pub fn local_browser_touch() -> Result<(), CrawlError> {
    let mut state = LOCAL_BROWSER.lock().map_err(|error| {
        CrawlError::InvalidInput(agena_failure::diagnostic::format_error_chain_with_context(
            "local browser registry mutex is poisoned",
            &error,
        ))
    })?;
    if let Some(browser) = state.browser.as_mut()
        && browser.is_running()?
    {
        state.last_used = Some(Instant::now());
    }
    Ok(())
}

/// Shut down the managed browser (if any) and remove its profile directory.
/// Returns `true` when a running browser was closed.
pub fn shutdown_local_browser() -> Result<bool, CrawlError> {
    let mut state = LOCAL_BROWSER.lock().map_err(|error| {
        CrawlError::InvalidInput(agena_failure::diagnostic::format_error_chain_with_context(
            "local browser registry mutex is poisoned",
            &error,
        ))
    })?;
    let running = match state.browser.as_mut() {
        Some(browser) => browser.is_running()?,
        None => false,
    };
    if running {
        tracing::debug!(target: "agena::web", "shutting down managed local browser");
    }
    // Move the browser out so process shutdown does not hold the registry
    // lock. Explicit shutdown returns cleanup failures to the caller; Drop is
    // only the logged last-resort retry.
    let browser = state.browser.take();
    state.last_used = None;
    state.idle_timeout = None;
    drop(state);
    if let Some(mut browser) = browser {
        browser.shutdown()?;
    }
    Ok(running)
}

fn kill_browser_tree(child: &mut Child) -> Result<(), CrawlError> {
    // Signal the main process first, then the process group on Unix / the
    // process tree on Windows so helper processes do not survive.
    let mut failures = Vec::new();
    if let Err(error) = child.kill() {
        failures.push(agena_failure::diagnostic::format_error_chain_with_context(
            "failed to kill the local browser process",
            &error,
        ));
    }
    #[cfg(unix)]
    {
        match Command::new("kill")
            .arg("-9")
            .arg(format!("-{}", child.id()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => failures.push(format!(
                "failed to kill local browser process group {}: kill exited with status {status}",
                child.id()
            )),
            Err(error) => {
                failures.push(agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to invoke kill for the local browser process group",
                    &error,
                ))
            }
        }
    }
    #[cfg(windows)]
    {
        match Command::new("taskkill")
            .arg("/PID")
            .arg(child.id().to_string())
            .arg("/T")
            .arg("/F")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => failures.push(format!(
                "failed to kill local browser process tree {}: taskkill exited with status {status}",
                child.id()
            )),
            Err(error) => failures.push(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to invoke taskkill for the local browser process tree",
                    &error,
                ),
            ),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else if matches!(child.try_wait(), Ok(Some(_))) {
        // Natural exit can race with shutdown and make the kill commands
        // report "not found" even though the desired state was reached.
        Ok(())
    } else {
        Err(CrawlError::InvalidInput(failures.join("; ")))
    }
}

fn shutdown_browser_process(child: &mut Child) -> Result<(), CrawlError> {
    // Do not block forever in wait if every termination strategy failed.
    // Drop will retry and log once the owner leaves scope.
    kill_browser_tree(child)?;
    child.wait().map(|_| ()).map_err(|error| {
        CrawlError::InvalidInput(agena_failure::diagnostic::format_error_chain_with_context(
            "failed to wait for the managed local browser process to exit",
            &error,
        ))
    })
}

fn wait_for_devtools_endpoint(
    child: &mut Child,
    profile_dir: &Path,
    timeout: Duration,
) -> Result<String, CrawlError> {
    let started = Instant::now();
    let active_port = profile_dir.join("DevToolsActivePort");
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(CrawlError::InvalidInput(format!(
                    "local browser exited before DevTools endpoint was ready: {status}"
                )));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(CrawlError::InvalidInput(
                    agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to inspect the local browser while waiting for its DevTools endpoint",
                        &error,
                    ),
                ));
            }
        }
        match fs::read_to_string(&active_port) {
            Ok(contents) => {
                let mut lines = contents.lines();
                if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                    let port = port.trim();
                    let path = path.trim();
                    if !port.is_empty() && !path.is_empty() {
                        return Ok(format!("ws://127.0.0.1:{port}{path}"));
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(CrawlError::InvalidInput(
                    agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to read the local browser DevTools endpoint file",
                        &error,
                    ),
                ));
            }
        }
        if started.elapsed() >= timeout {
            return Err(CrawlError::InvalidInput(format!(
                "timed out waiting for local browser DevTools endpoint after {}s",
                timeout.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

static IDLE_JANITOR_STARTED: Once = Once::new();

/// Start a lightweight daemon thread that closes the managed browser after the
/// configured idle timeout. The janitor is started once, on the first browser
/// launch that requests an idle timeout, and runs for the process lifetime.
fn ensure_idle_janitor() {
    IDLE_JANITOR_STARTED.call_once(|| {
        thread::spawn(|| {
            const CHECK_INTERVAL: Duration = Duration::from_secs(15);
            loop {
                thread::sleep(CHECK_INTERVAL);
                let should_shutdown = match LOCAL_BROWSER.lock() {
                    Ok(state) => match (state.idle_timeout, state.last_used) {
                        (Some(timeout), Some(last_used)) => {
                            Some(last_used.elapsed() >= timeout)
                        }
                        _ => None,
                    },
                    Err(error) => {
                        tracing::error!(
                            target: "agena::web",
                            diagnostic = %error,
                            "failed to inspect idle managed local browser because the registry mutex is poisoned"
                        );
                        None
                    }
                }
                .unwrap_or(false);
                if should_shutdown {
                    tracing::debug!(
                        target: "agena::web",
                        "closing idle managed local browser"
                    );
                    if let Err(error) = shutdown_local_browser() {
                        tracing::error!(
                            target: "agena::web",
                            error = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to shut down the idle managed local browser",
                                &error,
                            ),
                            "idle managed local browser cleanup failed"
                        );
                    }
                }
            }
        });
    });
}

fn find_browser_executable(configured: Option<&Path>) -> Result<PathBuf, CrawlError> {
    if let Some(path) = configured {
        if is_executable_candidate(path) {
            return Ok(path.to_path_buf());
        }
        return Err(CrawlError::InvalidInput(format!(
            "configured web.browser.executable_path '{}' is not executable",
            path.display()
        )));
    }

    match std::env::var("AGENA_CHROME_PATH") {
        Ok(path) => {
            let path = PathBuf::from(path);
            if is_executable_candidate(&path) {
                return Ok(path);
            }
        }
        Err(std::env::VarError::NotPresent) => {}
        Err(error) => {
            return Err(CrawlError::InvalidInput(format!(
                "failed to read AGENA_CHROME_PATH: {error}"
            )));
        }
    }

    for candidate in browser_candidates() {
        if is_executable_candidate(Path::new(candidate)) {
            return Ok(PathBuf::from(candidate));
        }
    }

    Err(CrawlError::InvalidInput(
        "local browser rendering requires Chrome/Chromium; set web.browser.executable_path or AGENA_CHROME_PATH".to_string(),
    ))
}

fn is_executable_candidate(path: &Path) -> bool {
    const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
    let Ok(mut child) = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    child
        .controlled()
        .time_limit(PROBE_TIMEOUT)
        .terminate_for_timeout()
        .wait()
        .ok()
        .flatten()
        .is_some_and(process_control::ExitStatus::success)
}

fn browser_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "google-chrome",
            "chromium",
        ]
    }
    #[cfg(target_os = "windows")]
    {
        &[
            "chrome.exe",
            "msedge.exe",
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        ]
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        &[
            "google-chrome-stable",
            "google-chrome",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_spawn_running_shutdown_round_trip() {
        // Chrome is an optional runtime dependency; skip when unavailable.
        let options = LocalBrowserOptions::default();
        let Ok(endpoint) = local_browser_endpoint(&options) else {
            return;
        };
        assert!(!endpoint.is_empty());
        assert!(local_browser_running().expect("inspect running browser"));
        assert!(shutdown_local_browser().unwrap_or_default());
        assert!(!local_browser_running().expect("inspect stopped browser"));
        // Shutting down again is a no-op and reports nothing was closed.
        assert!(!shutdown_local_browser().unwrap_or_default());
    }
}
