pub(crate) use agena_agent_browser::agena::ChromeSandbox;
use agena_agent_browser::agena::{ChromeProcess, launch_chrome};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use agena_plugin_host::{
    PluginError,
    sdk::{PluginErrorKind, Result as SdkResult},
};

#[derive(Debug, Clone)]
/// Options for launching a managed Chrome process.
pub(crate) struct ChromeOptions {
    sandbox: ChromeSandbox,
    pub executable_path: Option<PathBuf>,
    pub startup_timeout: Duration,
    /// How long the managed browser may stay idle before it is shut down
    /// automatically. `None` disables idle auto-close.
    pub idle_timeout: Option<Duration>,
}

impl ChromeOptions {
    pub(crate) fn new(
        executable_path: Option<&str>,
        startup_secs: u64,
        idle_secs: u64,
        sandbox: ChromeSandbox,
    ) -> Self {
        Self {
            sandbox,
            executable_path: executable_path.map(|path| PathBuf::from(path.trim())),
            startup_timeout: Duration::from_secs(startup_secs),
            idle_timeout: (idle_secs > 0).then(|| Duration::from_secs(idle_secs)),
        }
    }

    pub(crate) fn validate(&self) -> SdkResult<()> {
        if self
            .executable_path
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(PluginError::invalid_params(
                "Chrome executable_path must not be empty when set",
            ));
        }
        if self.startup_timeout.is_zero() {
            return Err(PluginError::invalid_params(
                "Chrome startup timeout must be greater than zero",
            ));
        }
        Ok(())
    }
}

fn start_browser(options: &ChromeOptions) -> SdkResult<ChromeProcess> {
    let executable = find_browser_executable(options.executable_path.as_deref())?;
    launch_chrome(&executable, options.startup_timeout, options.sandbox)
        .map_err(|error| PluginError::from_kind(PluginErrorKind::HostUnavailable, error))
}

/// A lazily started Chrome process owned by one plugin instance.
#[derive(Clone, Default)]
pub(crate) struct ChromeRuntime(Arc<BrowserState>);

#[derive(Default)]
struct BrowserState {
    process: Mutex<ProcessState>,
    activity: Mutex<BrowserActivity>,
    idle_janitor: OnceLock<()>,
}

#[derive(Default)]
struct ProcessState {
    browser: Option<ChromeProcess>,
    idle_timeout: Option<Duration>,
}

#[derive(Default)]
struct BrowserActivity {
    last_used: Option<Instant>,
    active_uses: usize,
}

fn lock<T>(state: &Mutex<T>) -> SdkResult<MutexGuard<'_, T>> {
    state.lock().map_err(|error| {
        PluginError::internal(format!("local Chrome state is unavailable: {error}"))
    })
}

impl ChromeRuntime {
    /// Start Chrome lazily and return its DevTools endpoint.
    pub(crate) async fn endpoint(&self, options: ChromeOptions) -> SdkResult<String> {
        let permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            runtime.start_or_reuse(&options)
        })
        .await
        .map_err(|error| PluginError::internal_error(&error))?
    }

    fn start_or_reuse(&self, options: &ChromeOptions) -> SdkResult<String> {
        let mut state = lock(&self.0.process)?;
        if state.browser.as_mut().is_none_or(ChromeProcess::has_exited) {
            state.browser = None;
            state.browser = Some(start_browser(options)?);
        }
        state.idle_timeout = options.idle_timeout;
        lock(&self.0.activity)?.last_used = Some(Instant::now());
        if options.idle_timeout.is_some() {
            self.start_idle_janitor();
        }
        Ok(state
            .browser
            .as_ref()
            .expect("Chrome initialized")
            .ws_url
            .clone())
    }

    pub(crate) async fn running(&self) -> SdkResult<bool> {
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || runtime.is_running())
            .await
            .map_err(|error| PluginError::internal_error(&error))?
    }

    fn is_running(&self) -> SdkResult<bool> {
        Ok(lock(&self.0.process)?
            .browser
            .as_mut()
            .is_some_and(|browser| !browser.has_exited()))
    }

    /// Prevent idle shutdown while a tool is using this browser.
    pub(crate) fn lease(&self) -> SdkResult<ChromeLease> {
        let mut activity = lock(&self.0.activity)?;
        activity.active_uses += 1;
        activity.last_used = Some(Instant::now());
        Ok(ChromeLease(self.0.clone()))
    }

    pub(crate) async fn shutdown(&self) -> SdkResult<()> {
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || runtime.stop())
            .await
            .map_err(|error| PluginError::internal_error(&error))?
    }

    fn stop(&self) -> SdkResult<()> {
        let mut state = lock(&self.0.process)?;
        let browser = state.browser.take();
        state.idle_timeout = None;
        lock(&self.0.activity)?.last_used = None;
        drop(state);
        drop(browser);
        Ok(())
    }

    fn start_idle_janitor(&self) {
        self.0.idle_janitor.get_or_init(|| {
            let browser = Arc::downgrade(&self.0);
            thread::spawn(move || {
                loop {
                    thread::sleep(Duration::from_secs(15));
                    let Some(state) = browser.upgrade() else {
                        return;
                    };
                    if let Err(error) = shutdown_if_idle(&state) {
                        tracing::error!(%error, "failed to close idle Chrome");
                    }
                }
            });
        });
    }
}

fn shutdown_if_idle(browser: &BrowserState) -> SdkResult<()> {
    // Launch and shutdown acquire the process state before the activity lock.
    // A tool lease only needs the activity lock, including on drop.
    let mut state = lock(&browser.process)?;
    let mut activity = lock(&browser.activity)?;
    if activity.active_uses != 0
        || !matches!(
            (state.idle_timeout, activity.last_used),
            (Some(timeout), Some(last_used)) if last_used.elapsed() >= timeout
        )
    {
        return Ok(());
    }
    let browser = state.browser.take();
    state.idle_timeout = None;
    activity.last_used = None;
    drop(activity);
    drop(state);
    drop(browser);
    Ok(())
}

pub(crate) struct ChromeLease(Arc<BrowserState>);

impl Drop for ChromeLease {
    fn drop(&mut self) {
        match lock(&self.0.activity) {
            Ok(mut activity) => {
                activity.active_uses = activity.active_uses.saturating_sub(1);
                activity.last_used = Some(Instant::now());
            }
            Err(error) => tracing::error!(%error, "failed to release Chrome lease"),
        }
    }
}

fn find_browser_executable(configured: Option<&Path>) -> SdkResult<PathBuf> {
    if let Some(path) = configured {
        if let Ok(resolved) = which::which(path) {
            return Ok(resolved);
        }
        return Err(PluginError::invalid_params(format!(
            "configured Chrome executable '{}' is not executable",
            path.display()
        )));
    }

    match std::env::var("AGENA_CHROME_PATH") {
        Ok(path) => {
            let path = PathBuf::from(path);
            if let Ok(resolved) = which::which(&path) {
                return Ok(resolved);
            }
        }
        Err(std::env::VarError::NotPresent) => {}
        Err(error) => {
            return Err(PluginError::internal(format!(
                "failed to read AGENA_CHROME_PATH: {error}"
            )));
        }
    }

    for candidate in browser_candidates() {
        if let Ok(resolved) = which::which(candidate) {
            return Ok(resolved);
        }
    }

    Err(PluginError::from_kind(
        PluginErrorKind::HostUnavailable,
        "Chrome or Chromium is required. Set executable_path in plugin settings or AGENA_CHROME_PATH.",
    ))
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
