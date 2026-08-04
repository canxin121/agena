use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex, Once};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::CrawlError;

#[derive(Debug, Clone)]
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
    profile_dir: PathBuf,
}

impl ManagedBrowser {
    fn spawn(options: &LocalBrowserOptions) -> Result<Self, CrawlError> {
        let executable = find_browser_executable(options.executable_path.as_deref())?;
        let profile_dir = browser_profile_dir();
        fs::create_dir_all(&profile_dir)?;
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
            .arg(format!("--user-data-dir={}", profile_dir.display()))
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

        let endpoint =
            match wait_for_devtools_endpoint(&mut child, &profile_dir, options.startup_timeout) {
                Ok(endpoint) => endpoint,
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = fs::remove_dir_all(&profile_dir);
                    return Err(err);
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
            profile_dir,
        })
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Kill the browser process tree and remove its temporary profile.
    fn shutdown(&mut self) {
        kill_browser_tree(&mut self.child);
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.profile_dir);
    }
}

impl Drop for ManagedBrowser {
    fn drop(&mut self) {
        self.shutdown();
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
    let mut state = LOCAL_BROWSER
        .lock()
        .map_err(|_| CrawlError::InvalidInput("local browser mutex poisoned".to_string()))?;
    if let Some(existing) = state.browser.as_mut()
        && existing.is_running()
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
pub fn local_browser_running() -> bool {
    LOCAL_BROWSER
        .lock()
        .ok()
        .map(|mut state| {
            state
                .browser
                .as_mut()
                .is_some_and(ManagedBrowser::is_running)
        })
        .unwrap_or(false)
}

/// Mark the managed browser as recently used so the idle auto-close timer
/// restarts. No-op when no browser is running.
pub fn local_browser_touch() {
    if let Ok(mut state) = LOCAL_BROWSER.lock() {
        if state
            .browser
            .as_mut()
            .is_some_and(ManagedBrowser::is_running)
        {
            state.last_used = Some(Instant::now());
        }
    }
}

/// Shut down the managed browser (if any) and remove its profile directory.
/// Returns `true` when a running browser was closed.
pub fn shutdown_local_browser() -> Result<bool, CrawlError> {
    let mut state = LOCAL_BROWSER
        .lock()
        .map_err(|_| CrawlError::InvalidInput("local browser mutex poisoned".to_string()))?;
    let running = state
        .browser
        .as_mut()
        .is_some_and(ManagedBrowser::is_running);
    if running {
        tracing::debug!(target: "agena::web", "shutting down managed local browser");
    }
    // Taking the value out of the option drops it while the lock is held;
    // `ManagedBrowser::drop` kills the child and removes the profile dir.
    state.browser = None;
    state.last_used = None;
    state.idle_timeout = None;
    Ok(running)
}

fn kill_browser_tree(child: &mut Child) {
    // Best effort: signal the main process first, then the process group on
    // Unix / the process tree on Windows so helper processes do not survive.
    let _ = child.kill();
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg("-9")
            .arg(format!("-{}", child.id()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(child.id().to_string())
            .arg("/T")
            .arg("/F")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn wait_for_devtools_endpoint(
    child: &mut Child,
    profile_dir: &Path,
    timeout: Duration,
) -> Result<String, CrawlError> {
    let started = Instant::now();
    let active_port = profile_dir.join("DevToolsActivePort");
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(CrawlError::InvalidInput(format!(
                "local browser exited before DevTools endpoint was ready: {status}"
            )));
        }
        if let Ok(contents) = fs::read_to_string(&active_port) {
            let mut lines = contents.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                let port = port.trim();
                let path = path.trim();
                if !port.is_empty() && !path.is_empty() {
                    return Ok(format!("ws://127.0.0.1:{port}{path}"));
                }
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
                let should_shutdown = LOCAL_BROWSER
                    .lock()
                    .ok()
                    .and_then(|state| {
                        let timeout = state.idle_timeout?;
                        let last_used = state.last_used?;
                        Some(last_used.elapsed() >= timeout)
                    })
                    .unwrap_or(false);
                if should_shutdown {
                    tracing::debug!(
                        target: "agena::web",
                        "closing idle managed local browser"
                    );
                    let _ = shutdown_local_browser();
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

    if let Ok(path) = std::env::var("AGENA_CHROME_PATH") {
        let path = PathBuf::from(path);
        if is_executable_candidate(&path) {
            return Ok(path);
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
    Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
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

fn browser_profile_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("agena-web-browser-{}-{nanos}", std::process::id()))
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
        assert!(local_browser_running());
        assert!(shutdown_local_browser().unwrap_or_default());
        assert!(!local_browser_running());
        // Shutting down again is a no-op and reports nothing was closed.
        assert!(!shutdown_local_browser().unwrap_or_default());
    }
}
