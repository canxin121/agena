//! Download completion comes from Chromium's GUID/state events, not from two
//! unchanged file sizes. A worker owns cancellation/cleanup and the global
//! download-behavior lease even if the waiting tool future is dropped.
use super::*;
#[derive(Debug)]
pub(super) struct DownloadResult {
    pub path: PathBuf,
    pub size: u64,
    pub guid: String,
    pub warnings: Vec<String>,
}
struct State {
    root: Option<CdpClient>,
    page: Option<CdpClient>,
    guid: Option<String>,
    changed_behavior: bool,
    dir: PathBuf,
    browser_context_id: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn transfer(
    endpoint: String,
    browser_context_id: Option<String>,
    target: String,
    url: String,
    dir: PathBuf,
    max_bytes: u64,
    timeout: Duration,
    gate: Arc<Mutex<()>>,
) -> SdkResult<DownloadResult> {
    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    let worker = tokio::spawn(async move {
        let permit = tokio::select! {biased; _=&mut cancel_rx=>return Err(PluginError::internal("download cancelled before admission")), result=tokio::time::timeout(timeout,gate.lock_owned())=>result.map_err(|_|PluginError::internal("download timed out waiting for admission"))?};
        let _permit = permit;
        let mut state = State {
            root: None,
            page: None,
            guid: None,
            changed_behavior: false,
            dir,
            browser_context_id,
        };
        let operation = receive(&endpoint, &target, &url, &mut state, max_bytes);
        let result = tokio::select! {biased; _=&mut cancel_rx=>Err(PluginError::internal("download cancelled; cleanup requested")), result=tokio::time::timeout(timeout,operation)=>result.unwrap_or_else(|_|Err(PluginError::internal("download deadline exceeded")))};
        let mut warnings = Vec::new();
        if result.is_err()
            && let Some(page) = state.page.as_ref()
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
                            serde_json::json!({"behavior":"default"}),
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
                result.warnings = warnings;
                Ok(result)
            }
            Err(error) if warnings.is_empty() => Err(error),
            Err(error) => Err(PluginError::internal(format!(
                "{}; cleanup warnings: {}",
                error.failure.user.fallback,
                warnings.join("; ")
            ))),
        }
    });
    let result = worker
        .await
        .map_err(|error| PluginError::internal(format!("download worker failed: {error}")))?;
    drop(cancel_tx);
    result
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
    Ok(Some(DownloadResult {
        path,
        size: metadata.len(),
        guid: guid.into(),
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
    target: &str,
    url: &str,
    state: &mut State,
    max_bytes: u64,
) -> SdkResult<DownloadResult> {
    let (tx, mut events) = mpsc::channel(512);
    let root = CdpClient::connect(endpoint, None, Some(tx)).await?;
    state.root = Some(root.clone());
    tokio::fs::create_dir_all(&state.dir)
        .await
        .map_err(|error| PluginError::internal_error(&error))?;
    state.changed_behavior = true;
    root.command("Browser.setDownloadBehavior",with_context(serde_json::json!({"behavior":"allowAndName","downloadPath":state.dir,"eventsEnabled":true}),state.browser_context_id.as_deref())).await?;
    let page = CdpClient::connect(endpoint, Some(target), None).await?;
    state.page = Some(page.clone());
    page.command("Page.enable", serde_json::json!({})).await?;
    let tree = page
        .command("Page.getFrameTree", serde_json::json!({}))
        .await?;
    let frame = tree
        .pointer("/frameTree/frame/id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| PluginError::internal("download target has no main frame"))?
        .to_owned();
    let navigation = page
        .command("Page.navigate", serde_json::json!({"url":url}))
        .await?;
    // ERR_ABORTED is expected when a navigation becomes a download. It is not
    // accepted as completion; the matching download events remain mandatory.
    if let Some(error) = navigation
        .get("errorText")
        .and_then(serde_json::Value::as_str)
        && !error.contains("ERR_ABORTED")
    {
        return Err(PluginError::internal(format!(
            "download navigation failed: {error}"
        )));
    }
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut completion_received = false;
    loop {
        tokio::select! {
            event=events.recv()=>{
                let event=event.ok_or_else(||PluginError::internal("download event stream closed before completion"))?;
                if event.method=="Browser.downloadWillBegin" {
                    let guid=event.params.get("guid").and_then(serde_json::Value::as_str).ok_or_else(||PluginError::internal("download event has no GUID"))?;
                    if event.params.get("frameId").and_then(serde_json::Value::as_str)!=Some(frame.as_str()) {continue;}
                    if state.guid.is_some() {return Err(PluginError::internal("one download request produced multiple downloads"));}
                    if guid.is_empty() || !guid.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-') {return Err(PluginError::internal("download GUID is not a safe artifact identifier"));}
                    state.guid=Some(guid.into());
                } else if event.method=="Browser.downloadProgress" && state.guid.as_deref()==event.params.get("guid").and_then(serde_json::Value::as_str) && state.guid.is_some() {
                    for field in ["totalBytes","receivedBytes"] {if event.params.get(field).and_then(serde_json::Value::as_f64).is_some_and(|bytes|bytes>max_bytes as f64) {return Err(PluginError::internal("in-flight browser download exceeds its byte limit"));}}
                    match event.params.get("state").and_then(serde_json::Value::as_str) {
                        Some("canceled")=>return Err(PluginError::internal("browser cancelled the download")),
                        Some("completed") => completion_received = true,
                        _=>{}
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

#[cfg(test)]
pub(super) async fn real_browser_regressions(
    endpoint: &str,
    target: &str,
    context_id: Option<String>,
) {
    use axum::{Router, routing::get};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let headers = [
        ("content-type", "application/octet-stream"),
        ("content-disposition", "attachment; filename=fixture.bin"),
    ];
    let app = Router::new()
        .route(
            "/small",
            get(move || async move { (headers, vec![b'a'; 8192]) }),
        )
        .route(
            "/big",
            get(move || async move { (headers, vec![b'b'; 128 * 1024]) }),
        )
        .route(
            "/slow",
            get(move || async move {
                let stream = futures_util::stream::unfold(0, |n| async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Some((Ok::<_, std::io::Error>(vec![b'c'; 1024]), n + 1))
                });
                (headers, axum::body::Body::from_stream(stream))
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let gate = Arc::new(Mutex::new(()));
    for attempt in 0..4 {
        let small = transfer(
            endpoint.into(),
            context_id.clone(),
            target.into(),
            format!("http://{address}/small"),
            dir.path().join(format!("small-{attempt}")),
            65536,
            Duration::from_secs(5),
            gate.clone(),
        )
        .await
        .unwrap();
        assert_eq!(small.size, 8192);
        assert_eq!(
            tokio::fs::read(&small.path).await.unwrap(),
            vec![b'a'; 8192]
        );
        assert!(!small.guid.is_empty());
    }
    let big_dir = dir.path().join("big");
    assert!(
        transfer(
            endpoint.into(),
            context_id.clone(),
            target.into(),
            format!("http://{address}/big"),
            big_dir.clone(),
            4096,
            Duration::from_secs(5),
            gate.clone()
        )
        .await
        .is_err()
    );
    assert!(
        !big_dir.exists(),
        "oversized download fixture should be cleaned"
    );
    let slow_dir = dir.path().join("slow");
    let endpoint = endpoint.to_owned();
    let target = target.to_owned();
    let g = gate.clone();
    let path = slow_dir.clone();
    let caller = tokio::spawn(async move {
        transfer(
            endpoint,
            context_id,
            target,
            format!("http://{address}/slow"),
            path,
            65536,
            Duration::from_secs(5),
            g,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    caller.abort();
    let _ = caller.await;
    let cleanup = tokio::time::timeout(Duration::from_secs(3), gate.lock_owned())
        .await
        .unwrap();
    drop(cleanup);
    assert!(
        !slow_dir.exists(),
        "cancelled caller must not leave an active partial download"
    );
    server.abort();
    eprintln!(
        "AUDIT_DOWNLOADS: GUID completion, in-flight limit and dropped-caller cleanup passed"
    );
}

#[cfg(test)]
mod file_visibility_tests {
    use super::*;
    #[tokio::test]
    async fn completion_waits_for_the_final_name_without_a_fixed_delay() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            completed_artifact(dir.path(), "guid", 1024)
                .await
                .unwrap()
                .is_none()
        );
        let partial = dir.path().join("guid.crdownload");
        tokio::fs::write(&partial, b"complete data").await.unwrap();
        assert!(
            completed_artifact(dir.path(), "guid", 1024)
                .await
                .unwrap()
                .is_none()
        );
        let mut entries = tokio::fs::read_dir(dir.path()).await.unwrap();
        let stale_entry = entries.next_entry().await.unwrap().unwrap();
        tokio::fs::rename(&partial, dir.path().join("guid"))
            .await
            .unwrap();
        assert!(
            artifact_metadata(&stale_entry.path())
                .await
                .unwrap()
                .is_none()
        );
        check_download_bytes(dir.path(), 1024).await.unwrap();
        let ready = completed_artifact(dir.path(), "guid", 1024)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready.size, 13);
        assert_eq!(
            tokio::fs::read(&ready.path).await.unwrap(),
            b"complete data"
        );
    }
    #[tokio::test]
    async fn waiting_for_visibility_does_not_waive_size_or_file_type_checks() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("guid"), b"oversized")
            .await
            .unwrap();
        assert!(completed_artifact(dir.path(), "guid", 4).await.is_err());
        assert!(check_download_bytes(dir.path(), 4).await.is_err());
        tokio::fs::remove_file(dir.path().join("guid"))
            .await
            .unwrap();
        tokio::fs::create_dir(dir.path().join("guid"))
            .await
            .unwrap();
        assert!(completed_artifact(dir.path(), "guid", 1024).await.is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("guid"), dir.path().join("link")).unwrap();
            assert!(completed_artifact(dir.path(), "link", 1024).await.is_err());
            assert!(check_download_bytes(dir.path(), 1024).await.is_err());
        }
    }
}
