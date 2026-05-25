use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::CrawlError;

#[derive(Debug, Clone)]
pub struct LocalBrowserOptions {
    pub executable_path: Option<PathBuf>,
    pub startup_timeout: Duration,
}

impl Default for LocalBrowserOptions {
    fn default() -> Self {
        Self {
            executable_path: None,
            startup_timeout: Duration::from_secs(10),
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
        let mut child = Command::new(&executable)
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
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| {
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
            target: "agena::crawl",
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
}

impl Drop for ManagedBrowser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.profile_dir);
    }
}

static LOCAL_BROWSER: LazyLock<Mutex<Option<ManagedBrowser>>> = LazyLock::new(|| Mutex::new(None));

pub fn local_browser_endpoint(options: &LocalBrowserOptions) -> Result<String, CrawlError> {
    let mut browser = LOCAL_BROWSER
        .lock()
        .map_err(|_| CrawlError::InvalidInput("local browser mutex poisoned".to_string()))?;
    if let Some(existing) = browser.as_mut()
        && existing.is_running()
    {
        return Ok(existing.endpoint.clone());
    }

    *browser = Some(ManagedBrowser::spawn(options)?);
    Ok(browser
        .as_ref()
        .expect("managed browser inserted")
        .endpoint
        .clone())
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

fn find_browser_executable(configured: Option<&Path>) -> Result<PathBuf, CrawlError> {
    if let Some(path) = configured {
        if is_executable_candidate(path) {
            return Ok(path.to_path_buf());
        }
        return Err(CrawlError::InvalidInput(format!(
            "configured crawl.browser_executable_path '{}' is not executable",
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
        "local browser rendering requires Chrome/Chromium; set crawl.browser_executable_path or AGENA_CHROME_PATH".to_string(),
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
    std::env::temp_dir().join(format!(
        "agena-crawl-browser-{}-{nanos}",
        std::process::id()
    ))
}
