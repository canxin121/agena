//! Download completion comes from Chromium's GUID/state events, not from two
//! unchanged file sizes. A worker owns cancellation/cleanup and the global
//! download-behavior lease even if the waiting tool future is dropped.
use super::*;
use agena_plugin_host::sdk::{PluginErrorKind, permission_wait};
use serde_json::{Value, json};
use tokio::sync::oneshot;

pub(super) enum DownloadTrigger {
    Navigate(String),
    Click {
        selector: String,
        frame_id: Option<String>,
    },
}

#[derive(Debug)]
pub(super) struct DownloadResult {
    pub path: PathBuf,
    pub size: u64,
    pub guid: String,
    pub filename: Option<String>,
    pub url: Option<String>,
    pub warnings: Vec<String>,
}
struct State {
    root: Option<BrowserDriver>,
    guid: Option<String>,
    filename: Option<String>,
    url: Option<String>,
    changed_behavior: bool,
    started: bool,
    dir: PathBuf,
    browser_context_id: Option<String>,
}

pub(super) async fn transfer(
    access: super::sessions::BrowserAccess,
    trigger: DownloadTrigger,
    dir: PathBuf,
    max_bytes: u64,
    timeout: Duration,
    gate: Arc<Mutex<()>>,
) -> SdkResult<DownloadResult> {
    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    let callback =
        agena_plugin_host::sdk::host_api::current_host_callback_context().unwrap_or_default();
    let worker = tokio::spawn(
        agena_plugin_host::sdk::host_api::run_in_isolated_host_callback_context(
            callback,
            async move {
                let start = tokio::time::Instant::now();
                let _permit = tokio::select! {
                    biased;
                    _ = &mut cancel_rx => return Err(cancelled()),
                    result = tokio::time::timeout(timeout, gate.lock_owned()) => {
                        result.map_err(|_| PluginError::from_kind(
                            PluginErrorKind::Timeout, "Download timed out waiting for admission",
                        ))?
                    }
                };
                let page = &access.page.driver;
                let mut state = State {
                    root: None,
                    guid: None,
                    filename: None,
                    url: None,
                    changed_behavior: false,
                    started: false,
                    dir,
                    browser_context_id: Some(access.page.context_id.clone()),
                };
                let remaining = timeout.saturating_sub(start.elapsed());
                let operation = receive(
                    &access.page.endpoint,
                    page,
                    &trigger,
                    &mut state,
                    max_bytes,
                    remaining,
                );
                let result = tokio::select! {
                    biased;
                    _ = &mut cancel_rx => Err(cancelled()),
                    result = permission_wait::timeout(remaining, operation) => result.unwrap_or_else(|_| {
                        Err(PluginError::from_kind(PluginErrorKind::Timeout, "Download deadline exceeded"))
                    }),
                };
                let mut warnings = Vec::new();
                if result.is_err() {
                    access.cancel_requests();
                }
                if result.is_err()
                    && state.started
                    && let Err(error) = page
                        .command_with_timeout(
                            "Page.stopLoading",
                            serde_json::json!({}),
                            Duration::from_secs(2),
                        )
                        .await
                {
                    warnings.push(format!(
                        "download navigation stop failed: {}",
                        error.failure.user.fallback
                    ));
                }
                if let Some(root) = state.root.as_ref() {
                    if result.is_err()
                        && let Some(guid) = state.guid.as_deref()
                        && let Err(error) = root
                            .command_with_timeout(
                                "Browser.cancelDownload",
                                with_context(
                                    serde_json::json!({"guid":guid}),
                                    state.browser_context_id.as_deref(),
                                ),
                                Duration::from_secs(2),
                            )
                            .await
                    {
                        warnings.push(format!(
                            "download cancellation failed: {}",
                            error.failure.user.fallback
                        ));
                    }
                    if state.changed_behavior
                        && let Err(error) = root
                            .command_with_timeout(
                                "Browser.setDownloadBehavior",
                                with_context(
                                    serde_json::json!({"behavior":"deny"}),
                                    state.browser_context_id.as_deref(),
                                ),
                                Duration::from_secs(2),
                            )
                            .await
                    {
                        warnings.push(format!(
                            "download behavior reset failed: {}",
                            error.failure.user.fallback
                        ));
                    }
                }
                if result.is_err() {
                    if warnings.is_empty()
                        && let Err(error) = tokio::fs::remove_dir_all(&state.dir).await
                        && error.kind() != std::io::ErrorKind::NotFound
                    {
                        warnings.push(format!("partial download cleanup failed: {error}"));
                    }
                    if !warnings.is_empty() {
                        tracing::error!(?warnings, directory=%state.dir.display(),"download cleanup needs attention");
                    }
                }
                match result {
                    Ok(mut result) => {
                        result.filename = state.filename;
                        result.url = state.url;
                        result.warnings = warnings;
                        Ok(result)
                    }
                    Err(mut error) => {
                        if !warnings.is_empty() {
                            error.diagnostic.message.push_str(&format!(
                                ". Download cleanup reported: {}",
                                warnings.join(". ")
                            ));
                        }
                        if state.started
                            && state.guid.is_none()
                            && matches!(
                                error.kind,
                                PluginErrorKind::Timeout
                                    | PluginErrorKind::Disconnected
                                    | PluginErrorKind::Cancelled
                            )
                        {
                            error = error.with_remote_effect_unknown();
                        }
                        Err(error)
                    }
                }
            },
        ),
    );
    let result = worker
        .await
        .map_err(|error| PluginError::internal(format!("download worker failed: {error}")))?;
    drop(cancel_tx);
    result
}

fn cancelled() -> PluginError {
    PluginError::from_kind(PluginErrorKind::Cancelled, "Browser download cancelled")
}
fn with_context(mut value: serde_json::Value, context: Option<&str>) -> serde_json::Value {
    if let Some(context) = context {
        value["browserContextId"] = serde_json::Value::String(context.into());
    }
    value
}

// A completed CDP event and a visible final file are separate conditions.
// Chromium can rename .crdownload after publishing completion or between a
// directory entry read and its stat. Only ENOENT is transient; other I/O
// failures and unsafe file types still fail the download.
async fn artifact_metadata(path: &Path) -> SdkResult<Option<std::fs::Metadata>> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(plugin_internal_error_with_context(
            "cannot inspect browser download artifact",
            &error,
        )),
    }
}

async fn completed_artifact(
    dir: &Path,
    guid: &str,
    max_bytes: u64,
) -> SdkResult<Option<DownloadResult>> {
    let path = dir.join(guid);
    let Some(metadata) = artifact_metadata(&path).await? else {
        return Ok(None);
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max_bytes {
        return Err(PluginError::internal(
            "completed download is not a bounded regular file",
        ));
    }
    agena_storage_layout::secure_file(&path)
        .map_err(|error| PluginError::internal_error(&error))?;
    Ok(Some(DownloadResult {
        path,
        size: metadata.len(),
        guid: guid.into(),
        filename: None,
        url: None,
        warnings: Vec::new(),
    }))
}

async fn check_download_bytes(dir: &Path, max_bytes: u64) -> SdkResult<()> {
    let mut entries = tokio::fs::read_dir(dir).await.map_err(|error| {
        plugin_internal_error_with_context("cannot inspect download directory", &error)
    })?;
    let mut bytes = 0_u64;
    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        plugin_internal_error_with_context("cannot enumerate download artifacts", &error)
    })? {
        let Some(metadata) = artifact_metadata(&entry.path()).await? else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            return Err(PluginError::internal(
                "browser download directory contains a symbolic link",
            ));
        }
        if metadata.is_file() {
            bytes = bytes.saturating_add(metadata.len());
        }
        if bytes > max_bytes {
            return Err(PluginError::internal(
                "partial download files exceed the byte limit",
            ));
        }
    }
    Ok(())
}

async fn receive(
    endpoint: &str,
    page: &BrowserDriver,
    trigger: &DownloadTrigger,
    state: &mut State,
    max_bytes: u64,
    timeout: Duration,
) -> SdkResult<DownloadResult> {
    let (tx, mut events) = mpsc::channel(512);
    let root = BrowserDriver::connect_root(endpoint, Some(tx)).await?;
    state.root = Some(root.clone());
    let directory = state.dir.clone();
    tokio::task::spawn_blocking(move || agena_storage_layout::secure_directory(&directory))
        .await
        .map_err(|error| PluginError::internal_error(&error))?
        .map_err(|error| PluginError::internal_error(&error))?;
    state.changed_behavior = true;
    root.command(
        "Browser.setDownloadBehavior",
        with_context(
            json!({"behavior": "allowAndName", "downloadPath": state.dir, "eventsEnabled": true}),
            state.browser_context_id.as_deref(),
        ),
    )
    .await?;
    let tree = page
        .command("Page.getFrameTree", serde_json::json!({}))
        .await?;
    let frame = tree
        .pointer("/frameTree/frame/id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| PluginError::internal("download target has no main frame"))?
        .to_owned();
    let mut frames = vec![frame];
    match trigger {
        DownloadTrigger::Navigate(url) => {
            state.started = true;
            let navigation = page.command("Page.navigate", json!({"url": url})).await?;
            // Chrome aborts HTML navigation when the response is an attachment.
            if let Some(error) = navigation["errorText"].as_str()
                && !error.contains("ERR_ABORTED")
            {
                return Err(PluginError::internal(format!(
                    "Download navigation failed: {error}"
                )));
            }
        }
        DownloadTrigger::Click { selector, frame_id } => {
            if let Some(frame_id) = frame_id {
                frames = vec![frame_id.clone()];
            } else {
                frames.extend(page.child_frame_ids().await?);
            }
            state.started = true;
            let result = page
                .action(
                    json!({"action": "click", "selector": selector, "frameId": frame_id}),
                    timeout,
                )
                .await?;
            if result["dialogOpened"] == true {
                return Err(PluginError::invalid_params(
                    "The click opened a dialog. Resolve it with browser.action.",
                ));
            }
        }
    }
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut completion_received = false;
    loop {
        tokio::select! {
            event = events.recv() => {
                let event = event.ok_or_else(|| PluginError::internal("Download event stream closed before completion"))?;
                let params = &event.params;
                if event.method == "Browser.downloadWillBegin" {
                    if params["frameId"].as_str().is_none_or(|frame| !frames.iter().any(|known| known == frame)) {
                        continue;
                    }
                    let guid = params["guid"].as_str().filter(|guid| {
                        !guid.is_empty() && guid.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    }).ok_or_else(|| PluginError::internal("Chrome returned an invalid download GUID"))?;
                    if state.guid.is_some() {
                        let _ = root.command("Browser.cancelDownload", with_context(
                            json!({"guid": guid}), state.browser_context_id.as_deref(),
                        )).await;
                        return Err(PluginError::internal("The trigger produced multiple downloads"));
                    }
                    state.guid = Some(guid.into());
                    state.filename = suggested_filename(params);
                    state.url = params["url"].as_str().map(|url| super::sessions::browser_text_prefix(url, 2048).to_owned());
                } else if state.guid.is_some() && state.guid.as_deref() == params["guid"].as_str() {
                    if ["totalBytes", "receivedBytes"].into_iter().any(|field| {
                        params[field].as_f64().is_some_and(|bytes| bytes > max_bytes as f64)
                    }) {
                        return Err(PluginError::internal("In-flight browser download exceeds its byte limit"));
                    }
                    match params["state"].as_str() {
                        Some("canceled") => return Err(PluginError::internal("Chrome cancelled the download")),
                        Some("completed") => completion_received = true,
                        _ => {}
                    }
                }
            }
            _=tick.tick()=>check_download_bytes(&state.dir, max_bytes).await?,
        }
        if completion_received
            && let Some(guid) = state.guid.as_deref()
            && let Some(result) = completed_artifact(&state.dir, guid, max_bytes).await?
        {
            return Ok(result);
        }
        // The original transfer timeout and cancellation still bound this
        // loop; no fresh deadline is granted for delayed file visibility.
    }
}

fn suggested_filename(params: &Value) -> Option<String> {
    let name = params["suggestedFilename"]
        .as_str()?
        .rsplit(['/', '\\'])
        .next()?;
    let name: String = name
        .chars()
        .filter(|ch| !ch.is_control())
        .take(200)
        .collect();
    (!matches!(name.trim(), "" | "." | "..")).then_some(name)
}
