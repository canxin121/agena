//! Optional, bounded Playwright actions on an already owned Chromium target.
//! The native connection continues to own contexts and navigation interception.
use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserInteractionBackend {
    #[default]
    Native,
    Playwright,
}

#[derive(Serialize)]
pub(super) struct Request<'a> {
    pub endpoint: &'a str,
    pub target_id: &'a str,
    pub context_id: &'a str,
    pub action: &'a str,
    pub selector: Option<&'a str>,
    pub frame_selector: Option<&'a str>,
    pub element_expression: Option<&'a str>,
    pub text: Option<&'a str>,
    pub press_enter: bool,
    pub timeout_ms: u64,
}

// Attaching to CDP initializes page agents. Serialize bridge connections so
// two short-lived Playwright drivers do not race their attachment/disposal.
static BRIDGES: Semaphore = Semaphore::const_new(1);

pub(super) async fn run(request: &Request<'_>) -> SdkResult<serde_json::Value> {
    let python = std::env::var_os("AGENA_BROWSER_PYTHON")
        .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into());
    run_with_python(request, python).await
}

async fn run_with_python(
    request: &Request<'_>,
    python: std::ffi::OsString,
) -> SdkResult<serde_json::Value> {
    if !(1..=120_000).contains(&request.timeout_ms) {
        return Err(PluginError::invalid_params(
            "browser timeout must be 1–120000 ms",
        ));
    }
    let bytes = serde_json::to_vec(request).map_err(|error| PluginError::internal_error(&error))?;
    if bytes.len() > 128 * 1024 {
        return Err(PluginError::invalid_params(
            "browser action exceeds 128 KiB request budget",
        ));
    }
    let timeout = Duration::from_millis(request.timeout_ms) + Duration::from_secs(5);
    let operation = async {
        let _permit = BRIDGES
            .acquire()
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let python = tokio::task::spawn_blocking(move || which::which(python))
            .await.map_err(|error| PluginError::internal_error(&error))?
            .map_err(|_| PluginError::invalid_params("Playwright needs a Python environment with playwright installed; set AGENA_BROWSER_PYTHON"))?;
        let mut command = tokio::process::Command::new(python);
        command.args([
            "-I",
            "-X",
            "utf8",
            "-c",
            include_str!("playwright/bridge.py"),
        ]);
        // Playwright debug call logs can contain filled values. Never inherit
        // logging/inspector configuration or Node injection into this bridge.
        for name in [
            "DEBUG",
            "DEBUG_FILE",
            "PWDEBUG",
            "NODE_OPTIONS",
            "PW_CHROMIUM_ATTACH_TO_OTHER",
        ] {
            command.env_remove(name);
        }
        let output = agena_process::output_with_input(command, &bytes, timeout, 16 * 1024)
            .await
            .map_err(|error| {
                plugin_internal_error_with_context("Playwright process failed", &error)
            })?;
        if !output.status.success() {
            // Neither stderr nor Python tracebacks are safe to echo here.
            return Err(PluginError::internal(format!(
                "Playwright bridge exited with {}; diagnostic text omitted to protect form data",
                output.status
            )));
        }
        decode(&output.stdout)
    };
    tokio::time::timeout(timeout, operation).await
        .map_err(|_| PluginError::internal("Playwright action timed out, including bridge admission/startup; refresh the snapshot before retrying"))?
}

fn decode(bytes: &[u8]) -> SdkResult<serde_json::Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Response {
        ok: bool,
        error: Option<String>,
    }
    let response: Response = serde_json::from_slice(bytes).map_err(|_| {
        PluginError::internal(
            "Playwright returned an invalid response; diagnostic text omitted to protect form data",
        )
    })?;
    if response.ok && response.error.is_none() {
        return Ok(serde_json::json!({"ok":true,"backend":"playwright"}));
    }
    let message = match response.error.as_deref() {
        Some("missing_dependency") => {
            "Playwright is unavailable in AGENA_BROWSER_PYTHON; install the playwright Python package in that environment"
        }
        Some("target_missing") => {
            "Playwright could not find the owned page and browser context; reopen the browser page"
        }
        Some("stale_reference") => {
            "Playwright rejected a stale browser reference; refresh the snapshot before acting"
        }
        Some("timeout") => {
            "Playwright actionability/readiness timed out; inspect the page for hidden, covered or disabled elements before retrying"
        }
        Some("ambiguous") => {
            "Playwright selector matched multiple elements; select one element or use a snapshot reference"
        }
        _ => {
            "Playwright action failed; refresh the snapshot and check that the selected element supports this action (details omitted to protect form data)"
        }
    };
    Err(PluginError::internal(message))
}

#[cfg(test)]
mod tests;
