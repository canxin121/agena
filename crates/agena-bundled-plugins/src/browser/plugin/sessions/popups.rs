use super::*;
use agena_plugin_host::sdk::permission_wait;
use serde_json::{Value, json};
use tokio::task::JoinSet;

pub(super) async fn watch(
    state: &Arc<BrowserSessions>,
    root: &BrowserDriver,
    host: Arc<dyn HostClient>,
    capture_console: bool,
) -> SdkResult<()> {
    let mut watcher = root.watch_new_pages().await?;
    let weak_state = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut adoptions = JoinSet::new();
        loop {
            tokio::select! {
                _ = adoptions.join_next(), if !adoptions.is_empty() => {}
                event = watcher.next() => {
                    let Some((root, event)) = event else { break };
                    let Some(state) = weak_state.upgrade() else { break };
                    let info = &event.params["targetInfo"];
                    let Some(id) = info["targetId"].as_str() else { continue };
                    let Some(session) = event.params["sessionId"].as_str() else { continue };
                    if info["openerId"].is_null() {
                        if event.params["waitingForDebugger"] == true {
                            // Agena creates explicit targets at about:blank, then attaches
                            // their request policy before issuing the initial navigation.
                            let result = if info["url"] == "about:blank" {
                                root.resume_new_page(session).await
                            } else {
                                close_unmanaged(&root, id).await
                            };
                            if let Err(error) = result {
                                log_secondary_plugin_failure("initialize browser target", &error);
                                let _ = close_unmanaged(&root, id).await;
                            }
                        }
                        continue;
                    }
                    let opening = OpeningTarget { root: root.clone(), target: Some(id.to_owned()) };
                    if event.params["waitingForDebugger"] != true || adoptions.len() >= 16 {
                        drop(opening);
                        continue;
                    }
                    let info = info.clone();
                    let session = session.to_owned();
                    let host = host.clone();
                    adoptions.spawn(async move {
                        if let Err(error) = adopt(state, opening, &session, &info, host, capture_console).await {
                            log_secondary_plugin_failure("adopt browser popup", &error);
                        }
                    });
                }
            }
        }
    });
    Ok(())
}

async fn close_unmanaged(root: &BrowserDriver, id: &str) -> SdkResult<()> {
    root.command_with_timeout(
        "Target.closeTarget",
        json!({"targetId": id}),
        Duration::from_secs(5),
    )
    .await
    .map(|_| ())
}

async fn adopt(
    state: Arc<BrowserSessions>,
    mut opening: OpeningTarget,
    paused_session: &str,
    info: &Value,
    host: Arc<dyn HostClient>,
    capture_console: bool,
) -> SdkResult<()> {
    let Some(opener) = find_opener(&state, info).await else {
        return Ok(());
    };
    if opener.preview.is_some() {
        return Ok(());
    }
    let Some(authority) = opener.driver.popup_scope() else {
        return Ok(());
    };
    let id = opening.target.as_deref().expect("paused target").to_owned();
    let (sender, events) = mpsc::channel(512);
    let driver = permission_wait::timeout(
        Duration::from_secs(5),
        BrowserDriver::connect_page_with_popup(
            &opener.endpoint,
            &id,
            PageEvents::Activity {
                sender,
                capture_console,
                allow_popups: true,
            },
            Some(&authority),
        ),
    )
    .await
    .map_err(|_| {
        PluginError::from_kind(PluginErrorKind::Timeout, "Browser popup setup timed out")
    })??;
    if !authority.is_active() {
        return Ok(());
    }
    let url = agena_plugin_host::sdk::host_api::run_in_isolated_host_callback_context(
        authority.callback_context(),
        async {
            permission_wait::timeout(Duration::from_secs(5), async {
                tokio::try_join!(
                    opening.root.resume_new_page(paused_session),
                    driver.enable_popup_page()
                )?;
                initial_url(&driver).await
            })
            .await
            .map_err(|_| {
                PluginError::from_kind(
                    PluginErrorKind::Timeout,
                    "Browser popup did not become ready",
                )
            })?
        },
    )
    .await?;
    if !authority.is_active() || driver.is_closed() {
        return Ok(());
    }
    let title = url::Url::parse(&url)
        .ok()
        .filter(|url| url.has_host())
        .map_or_else(
            || "Browser popup".to_owned(),
            |url| format!("Browser · {}", browser_title_target(&url)),
        );
    let mut log = BrowserSessionLog::new();
    let public_url = if authority.is_human() {
        url::Url::parse(&url)
            .ok()
            .filter(|url| url.has_host())
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_default()
    } else {
        url.clone()
    };
    let title = if authority.is_human() {
        "Browser tab".into()
    } else {
        title
    };
    log.append("event", format!("Opened from browser page {}.", opener.id));
    let page = Arc::new(BrowserPage {
        id: id.clone(),
        owner: opener.owner.clone(),
        opener_id: Some(opener.id.clone()),
        context_id: opener.context_id.clone(),
        endpoint: opener.endpoint.clone(),
        driver,
        preview: None,
        uploads: Mutex::default(),
        root: opening.root.clone(),
        gate: Arc::new(Mutex::new(())),
        control: opener.control.clone(),
        activity: Mutex::new(PageActivity {
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            title,
            url: public_url,
            status: None,
            log,
        }),
    });
    page.verify_url(&url).await?;
    {
        let mut pages = state.pages.lock().await;
        if !pages.contains_key(&opener.id) || !authority.is_active() {
            return Ok(());
        }
        pages.insert(id.clone(), page.clone());
    }
    opening.target = None;
    state.observe(&page, events, host.clone(), capture_console);
    authority.opened(id);
    page.publish(BackgroundActivityStatus::Running, None, host.as_ref())
        .await;
    Ok(())
}

async fn find_opener(state: &BrowserSessions, info: &Value) -> Option<Arc<BrowserPage>> {
    let context = info["browserContextId"].as_str()?;
    let opener_id = info["openerId"].as_str()?;
    let pages = {
        let pages = state.pages.lock().await;
        if let Some(page) = pages
            .get(opener_id)
            .filter(|page| page.context_id == context)
        {
            return Some(page.clone());
        }
        pages
            .values()
            .filter(|page| page.context_id == context)
            .cloned()
            .collect::<Vec<_>>()
    };
    let frame = info["openerFrameId"].as_str()?;
    for page in pages {
        if page
            .driver
            .child_frame_ids()
            .await
            .is_ok_and(|ids| ids.iter().any(|id| id == frame))
        {
            return Some(page);
        }
    }
    None
}

async fn initial_url(driver: &BrowserDriver) -> SdkResult<String> {
    let started = tokio::time::Instant::now();
    permission_wait::timeout(Duration::from_secs(4), async {
        loop {
            let tree = driver.command("Page.getFrameTree", json!({})).await?;
            if let Some(url) = tree.pointer("/frameTree/frame/url").and_then(Value::as_str) {
                if url.starts_with("http://") || url.starts_with("https://") {
                    let ready = driver.evaluate("document.readyState").await?;
                    if matches!(ready.as_str(), Some("interactive" | "complete")) {
                        return Ok(url.to_owned());
                    }
                }
                if url == "about:blank"
                    && started.elapsed() >= Duration::from_millis(250)
                    && !driver.saw_document_request()?
                {
                    return Ok(url.to_owned());
                }
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .map_err(|_| {
        PluginError::from_kind(
            PluginErrorKind::Timeout,
            "The popup document did not commit",
        )
    })?
}
