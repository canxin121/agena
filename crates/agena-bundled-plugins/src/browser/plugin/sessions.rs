//! Session ownership and tool transactions over the embedded browser engine.

#[cfg(all(test, unix))]
mod chrome_tests;
pub(super) mod handoff;
mod popups;

use super::*;
use crate::browser::driver::{
    BrowserAuthorization, BrowserDriver, BrowserRequestPolicy, CHROME_ERROR_DOCUMENT, CdpEvent,
    PageEvents,
};
use crate::browser::runtime::ChromeLease;
use agena_plugin_host::sdk::PluginErrorKind;
use tokio::sync::{OwnedMutexGuard, mpsc};

type BrowserOwner = agena_runtime_tools::TerminalOwner;

#[derive(Default)]
pub(super) struct BrowserSessions {
    local: ChromeRuntime,
    activity_host: Option<Arc<dyn HostClient>>,
    plugin_generations: std::sync::Mutex<std::sync::Weak<()>>,
    root: Mutex<Option<BrowserRoot>>,
    pages: Mutex<BTreeMap<String, Arc<BrowserPage>>>,
    pub(super) download_gate: Arc<Mutex<()>>,
    pub(super) content: Mutex<super::browser_tools::ContentCache>,
    controls: std::sync::Mutex<BTreeMap<i64, std::sync::Weak<handoff::Control>>>,
}

struct BrowserRoot {
    endpoint: String,
    driver: BrowserDriver,
    contexts: BTreeMap<(PathBuf, Option<i64>), String>,
}

pub(super) struct BrowserPage {
    pub(super) id: String,
    owner: BrowserOwner,
    opener_id: Option<String>,
    pub(super) context_id: String,
    pub(super) endpoint: String,
    pub(super) driver: BrowserDriver,
    pub(super) preview: Option<ManagedPreviewTarget>,
    pub(super) uploads: Mutex<super::uploads::UploadStore>,
    root: BrowserDriver,
    gate: Arc<Mutex<()>>,
    control: Arc<handoff::Control>,
    activity: Mutex<PageActivity>,
}

struct PageActivity {
    started_at_ms: i64,
    title: String,
    url: String,
    status: Option<BackgroundActivityStatus>,
}

/// Keep the action gate, Chrome lease, and callback identity until the last
/// command of a tool call has finished, including download cleanup.
pub(super) struct BrowserAccess {
    pub(super) page: Arc<BrowserPage>,
    _authorization: BrowserAuthorization,
    _lease: ChromeLease,
    _gate: OwnedMutexGuard<()>,
    _control: handoff::AgentAccess,
}

impl BrowserAccess {
    pub(super) fn cancel_requests(&self) {
        self._authorization.cancel_requests();
    }
}

impl BrowserSessions {
    pub(super) fn with_activity_host(host: Arc<dyn HostClient>) -> Self {
        Self {
            activity_host: Some(host),
            ..Self::default()
        }
    }

    pub(super) fn activity_host(&self, fallback: &Arc<dyn HostClient>) -> Arc<dyn HostClient> {
        self.activity_host.as_ref().unwrap_or(fallback).clone()
    }

    pub(super) fn attach_plugin(&self) -> Arc<()> {
        let mut generations = self
            .plugin_generations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        generations.upgrade().unwrap_or_else(|| {
            let lease = Arc::new(());
            *generations = Arc::downgrade(&lease);
            lease
        })
    }

    pub(super) fn lease(&self) -> SdkResult<ChromeLease> {
        self.local.lease()
    }

    async fn owned_page(&self, owner: &BrowserOwner, target: &str) -> SdkResult<Arc<BrowserPage>> {
        self.pages.lock().await.get(target).filter(|page| &page.owner == owner).cloned()
            .ok_or_else(|| PluginError::invalid_params("Browser page not found in this Agena session. Call browser.open to open a page."))
    }

    async fn lock_page(&self, page: &Arc<BrowserPage>) -> SdkResult<OwnedMutexGuard<()>> {
        let guard = page.gate.clone().lock_owned().await;
        if !self.pages.lock().await.contains_key(&page.id) {
            return Err(PluginError::from_kind(
                PluginErrorKind::Disconnected,
                "The browser page has closed. Call browser.open to open a page.",
            ));
        }
        Ok(guard)
    }

    async fn close(
        &self,
        page: &Arc<BrowserPage>,
        status: BackgroundActivityStatus,
        message: &str,
        host: &dyn HostClient,
    ) -> SdkResult<()> {
        let _gate = self.lock_page(page).await?;
        self.close_locked(page, status, message, host).await
    }

    pub(super) async fn close_locked(
        &self,
        page: &BrowserPage,
        status: BackgroundActivityStatus,
        message: &str,
        host: &dyn HostClient,
    ) -> SdkResult<()> {
        if !page.root.is_closed() {
            match page
                .root
                .command_with_timeout(
                    "Target.closeTarget",
                    serde_json::json!({"targetId": page.id}),
                    Duration::from_secs(5),
                )
                .await
            {
                Ok(result) if result["success"] == true => {}
                Err(error)
                    if error
                        .diagnostic_message()
                        .contains("No target with given id") => {}
                Err(error) if !page.root.is_closed() => return Err(error),
                Ok(_) => {
                    return Err(PluginError::internal(
                        "Chrome did not acknowledge closing the page",
                    ));
                }
                Err(_) => {}
            }
        }
        // A lost root connection releases its disposable contexts in Chrome.
        self.pages.lock().await.remove(&page.id);
        self.content.lock().await.remove_page(&page.id);
        page.publish(status, Some(message), host).await;
        Ok(())
    }

    pub(super) async fn shutdown(&self, host: &dyn HostClient) -> SdkResult<()> {
        let root = self.root.lock().await.take();
        let pages = std::mem::take(&mut *self.pages.lock().await);
        *self.content.lock().await = Default::default();
        for page in pages.values() {
            page.publish(
                BackgroundActivityStatus::Stopped,
                Some("Browser plugin stopped."),
                host,
            )
            .await;
        }
        drop(pages);
        drop(root);
        self.local.shutdown().await
    }
}

impl BrowserPage {
    async fn publish(
        &self,
        status: BackgroundActivityStatus,
        message: Option<&str>,
        host: &dyn HostClient,
    ) {
        let mut meta = self.activity.lock().await;
        if meta.status.is_some_and(|status| status.is_terminal()) {
            return;
        }
        let mut activity = browser_activity(
            format!("browser_{}", self.id),
            meta.title.clone(),
            meta.url.clone(),
            status,
            Some(meta.started_at_ms),
            status
                .is_terminal()
                .then(|| chrono::Utc::now().timestamp_millis()),
            message.map(str::to_owned),
        );
        activity.session_id = self.owner.session_id;
        meta.status = Some(status);
        if let Err(error) = host.publish_activity(activity).await {
            log_secondary_plugin_failure("publish browser activity", &error);
        }
    }

    pub(super) async fn verify_preview_lease(&self) -> SdkResult<()> {
        if let Some(preview) = &self.preview {
            preview.validate().await.map_err(managed_preview::denied)?;
        }
        Ok(())
    }

    async fn verify_url(&self, raw: &str) -> SdkResult<()> {
        if raw == CHROME_ERROR_DOCUMENT {
            return Err(self
                .driver
                .navigation_error(&self.driver.main_frame().await?));
        }
        if raw == "about:blank" && self.opener_id.is_some() && self.preview.is_none() {
            return Ok(());
        }
        let url = url::Url::parse(raw).map_err(|error| PluginError::internal_error(&error))?;
        if let Some(preview) = &self.preview {
            return preview
                .validate_url(&url)
                .await
                .map_err(managed_preview::denied);
        }
        network_policy::validate_proxy_target(&url)
    }

    pub(super) async fn committed_url(&self) -> SdkResult<String> {
        let frame = self.driver.main_frame().await?;
        let url = frame["url"].as_str().expect("main frame URL checked");
        if url == CHROME_ERROR_DOCUMENT || frame["unreachableUrl"].is_string() {
            return Err(self.driver.navigation_error(&frame));
        }
        Ok(format!(
            "{url}{}",
            frame["urlFragment"].as_str().unwrap_or_default()
        ))
    }

    pub(super) async fn verify_committed_url(&self) -> SdkResult<()> {
        self.verify_url(&self.committed_url().await?).await
    }

    pub(super) async fn verify_navigation_scope(&self) -> SdkResult<()> {
        let frame = self.driver.main_frame().await?;
        let url = frame["url"].as_str().expect("main frame URL checked");
        if url == CHROME_ERROR_DOCUMENT {
            return match (&self.preview, frame["unreachableUrl"].as_str()) {
                (None, _) => Ok(()),
                (Some(_), Some(attempted)) => self.verify_url(attempted).await,
                (Some(_), None) => Err(self.driver.navigation_error(&frame)),
            };
        }
        self.verify_url(url).await
    }

    async fn snapshot(&self, options: &SnapshotOptions) -> SdkResult<BrowserSnapshot> {
        let snapshot = self.driver.action(serde_json::json!({
            "action": "snapshot", "compact": true, "omitValues": true,
            "interactive": options.interactive, "selector": options.selector, "maxDepth": options.max_depth,
            "frameId": options.frame_id,
        }), Duration::from_secs(30)).await?;
        let url = snapshot["url"]
            .as_str()
            .ok_or_else(|| PluginError::internal("Chrome did not report a snapshot URL"))?;
        self.verify_url(url).await?;
        let tree = snapshot["snapshot"]
            .as_str()
            .ok_or_else(|| PluginError::internal("Chrome did not return an accessibility tree"))?;
        let prefix = browser_text_prefix(tree, 30_000);
        let truncated = prefix.len() < tree.len();
        let tree = if truncated {
            prefix.rsplit_once('\n').map_or("", |(head, _)| head)
        } else {
            prefix
        };
        Ok(BrowserSnapshot {
            title: snapshot["title"].as_str().unwrap_or_default().to_owned(),
            url: url.to_owned(),
            tree: tree.to_owned(),
            truncated,
        })
    }

    async fn publish_snapshot(&self, snapshot: &BrowserSnapshot, host: &dyn HostClient) {
        let mut meta = self.activity.lock().await;
        let title = if snapshot.title.trim().is_empty() {
            meta.title.clone()
        } else {
            format!("Browser · {}", browser_text_prefix(&snapshot.title, 512))
        };
        if meta.status.is_some() && meta.title == title && meta.url == snapshot.url {
            return;
        }
        meta.url.clone_from(&snapshot.url);
        meta.title = title;
        drop(meta);
        self.publish(BackgroundActivityStatus::Running, None, host)
            .await;
    }
}

struct SessionBrowserController {
    state: std::sync::Weak<BrowserSessions>,
    host: Arc<dyn HostClient>,
    fake_ip_ranges: Vec<String>,
}

#[async_trait]
impl BrowserController for SessionBrowserController {
    async fn browser_control(
        &self,
        session_id: i64,
        command: agena_plugin_sdk::browser::BrowserControlCommand,
        network: Option<Arc<dyn agena_plugin_sdk::browser::BrowserNetworkAuthority>>,
    ) -> SdkResult<agena_plugin_sdk::browser::BrowserControlResult> {
        let state = self.state.upgrade().ok_or_else(handoff::closed)?;
        state
            .operator_command(
                session_id,
                command,
                network,
                self.host.clone(),
                &self.fake_ip_ranges,
            )
            .await
    }
}

/// Close an incompletely opened page even if the invoking tool was cancelled.
struct OpeningTarget {
    root: BrowserDriver,
    target: Option<String>,
}

impl OpeningTarget {
    async fn create(root: &BrowserDriver, context: &str) -> SdkResult<Self> {
        let root = root.clone();
        let context = context.to_owned();
        // Dropping the returned guard closes a target created after caller cancellation.
        tokio::spawn(async move {
            let target = root.create_blank_target(&context).await?;
            Ok(Self {
                root,
                target: Some(target),
            })
        })
        .await
        .map_err(|error| PluginError::internal_error(&error))?
    }
}

impl Drop for OpeningTarget {
    fn drop(&mut self) {
        let Some(target) = self.target.take() else {
            return;
        };
        let root = self.root.clone();
        tokio::spawn(async move {
            if let Err(error) = root
                .command(
                    "Target.closeTarget",
                    serde_json::json!({"targetId": target}),
                )
                .await
            {
                log_secondary_plugin_failure("close incomplete browser page", &error);
            }
        });
    }
}

pub(super) fn browser_owner(context: &ToolInvokeContext<'_>) -> SdkResult<BrowserOwner> {
    Ok(BrowserOwner {
        workspace: if context.is_global() {
            PathBuf::new()
        } else {
            Path::new(context.workspace_root)
                .canonicalize()
                .map_err(|error| PluginError::internal_error(&error))?
        },
        session_id: Some(context.session_id),
    })
}

impl BrowserPlugin {
    async fn ensure_browser_controller(&self, control: &handoff::ControlState) -> SdkResult<()> {
        let host = self.state()?.host.clone();
        let fake_ip_ranges = self.config()?.fake_ip_ranges.clone();
        control
            .controller
            .get_or_try_init(|| async {
                let controller: Arc<dyn BrowserController> = Arc::new(SessionBrowserController {
                    state: Arc::downgrade(&self.browser_state),
                    host: self.browser_state.activity_host(&host),
                    fake_ip_ranges,
                });
                host.register_browser_controller(controller.clone()).await?;
                Ok(controller)
            })
            .await
            .map(|_| ())
    }

    pub(super) async fn browser_access(
        &self,
        context: &ToolInvokeContext<'_>,
        target: &str,
    ) -> SdkResult<BrowserAccess> {
        let control = handoff::agent_access(self.browser_state.control(context.session_id)).await?;
        let page = self
            .browser_state
            .owned_page(&browser_owner(context)?, target)
            .await?;
        let gate = self.browser_state.lock_page(&page).await?;
        let lease = self.browser_state.lease()?;
        if let Some(preview) = &page.preview {
            managed_preview::require_upstream_permission(preview, self.state()?.host.as_ref())
                .await?;
            page.verify_navigation_scope().await?;
        }
        let authorization = page
            .driver
            .authorize(self.browser_request_policy(page.preview.clone())?)?;
        Ok(BrowserAccess {
            page,
            _authorization: authorization,
            _lease: lease,
            _gate: gate,
            _control: control,
        })
    }

    pub(super) async fn validate_browser_destination(
        &self,
        preview: Option<&ManagedPreviewTarget>,
        url: &url::Url,
    ) -> SdkResult<()> {
        if managed_preview::authorize_request(preview, url, true, self.state()?.host.as_ref())
            .await?
        {
            return Ok(());
        }
        network_policy::require_browser_target(
            url,
            &self.config()?.fake_ip_ranges,
            self.state()?.host.as_ref(),
        )
        .await
    }

    fn browser_request_policy(
        &self,
        preview: Option<ManagedPreviewTarget>,
    ) -> SdkResult<BrowserRequestPolicy> {
        let host = self.state()?.host.clone();
        let fake_ip_ranges = self.config()?.fake_ip_ranges.clone();
        Ok(Arc::new(move |raw, is_document| {
            let host = host.clone();
            let fake_ip_ranges = fake_ip_ranges.clone();
            let preview = preview.clone();
            Box::pin(async move {
                let url = prepare_fetch_url(&raw).map_err(PluginError::invalid_params)?;
                if managed_preview::authorize_request(
                    preview.as_ref(),
                    &url,
                    is_document,
                    host.as_ref(),
                )
                .await?
                {
                    return Ok(());
                }
                network_policy::require_browser_target(&url, &fake_ip_ranges, host.as_ref()).await
            })
        }))
    }

    pub(super) async fn open_browser(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserOpenInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let control = self.browser_state.control(context.session_id);
        let agent_access = handoff::agent_access(control.clone()).await?;
        let owner = browser_owner(context)?;
        let (url, preview) = match managed_preview::open_target(input)? {
            managed_preview::OpenTarget::Public(raw) => (
                prepare_fetch_url(raw).map_err(PluginError::invalid_params)?,
                None,
            ),
            managed_preview::OpenTarget::Preview(id) => {
                let preview = agena_web::managed_preview::resolve_managed_preview(
                    &owner.workspace,
                    &context.session_id.to_string(),
                    id,
                )
                .await
                .map_err(managed_preview::denied)?;
                (preview.url().clone(), Some(preview))
            }
        };
        self.validate_browser_destination(preview.as_ref(), &url)
            .await?;
        self.ensure_browser_controller(&agent_access).await?;
        let _lease = self.browser_state.lease()?;
        let host = self.browser_state.activity_host(&self.state()?.host);
        let capture_console = self.config()?.capture_console_text;
        let (page, mut opening, event_rx) = {
            let mut root = self.browser_state.root.lock().await;
            let endpoint = self
                .browser_state
                .local
                .endpoint(self.config()?.chrome_options())
                .await?;
            if root
                .as_ref()
                .is_none_or(|root| root.endpoint != endpoint || root.driver.is_closed())
            {
                let driver = BrowserDriver::connect_root(&endpoint, None).await?;
                popups::watch(&self.browser_state, &driver, host.clone(), capture_console).await?;
                *root = Some(BrowserRoot {
                    driver,
                    endpoint,
                    contexts: BTreeMap::new(),
                });
            }
            let root = root.as_mut().expect("Chrome root initialized");
            let context_id = root.context_for(&owner).await?;
            let opening = OpeningTarget::create(&root.driver, &context_id).await?;
            let id = opening.target.clone().expect("newly opened target");
            let (events, event_rx) = mpsc::channel(512);
            let driver = BrowserDriver::connect_page(
                &root.endpoint,
                &id,
                PageEvents::Activity {
                    sender: events,
                    capture_console,
                    allow_popups: preview.is_none(),
                },
            )
            .await?;
            let page = Arc::new(BrowserPage {
                id,
                owner,
                opener_id: None,
                context_id,
                endpoint: root.endpoint.clone(),
                driver,
                preview,
                uploads: Mutex::default(),
                root: root.driver.clone(),
                gate: Arc::new(Mutex::new(())),
                control,
                activity: Mutex::new(PageActivity {
                    started_at_ms: chrono::Utc::now().timestamp_millis(),
                    title: format!("Browser · {}", browser_title_target(&url)),
                    url: url.to_string(),
                    status: None,
                }),
            });
            (page, opening, event_rx)
        };
        let _authorization = page
            .driver
            .authorize(self.browser_request_policy(page.preview.clone())?)?;
        page.driver
            .action_and_settle(
                navigation_command(&url, input.timeout_ms),
                Duration::from_millis(input.timeout_ms),
            )
            .await?;
        let snapshot = page.snapshot(&SnapshotOptions::default()).await?;
        let _gate = page.gate.lock().await;
        {
            let root = self.browser_state.root.lock().await;
            let key = (page.owner.workspace.clone(), page.owner.session_id);
            if page.driver.is_closed()
                || root.as_ref().is_none_or(|root| {
                    root.endpoint != page.endpoint
                        || root.contexts.get(&key) != Some(&page.context_id)
                })
            {
                return Err(PluginError::from_kind(
                    PluginErrorKind::Disconnected,
                    "The browser context closed while opening the page",
                ));
            }
            self.browser_state
                .pages
                .lock()
                .await
                .insert(page.id.clone(), page.clone());
        }
        self.browser_state.observe(&page, event_rx, host.clone());
        page.publish_snapshot(&snapshot, host.as_ref()).await;
        opening.target = None;
        Ok(snapshot.output("Open browser page", serde_json::json!({
            "session_id": page.id,
            "preview_id": page.preview.as_ref().map(ManagedPreviewTarget::preview_id),
            "preview_generation": page.preview.as_ref().map(ManagedPreviewTarget::generation),
            "managed_preview_network_policy": page.preview.as_ref().map(|_| managed_preview::POLICY_DESCRIPTION),
            "snapshot": snapshot,
        })))
    }

    pub(super) async fn list_browsers(
        &self,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        let control = self.browser_state.control(context.session_id);
        let control = control.read().await;
        if let Some(status) = control.waiting_status() {
            return Ok(browser_output(
                "List browser pages",
                status.into(),
                serde_json::json!({"status": "waiting_for_user", "message": status}),
            ));
        }
        let owner = browser_owner(context)?;
        let previews = agena_web::managed_preview::list_managed_previews(
            &owner.workspace,
            &context.session_id.to_string(),
        )
        .await
        .map_err(managed_preview::denied)?;
        let pages = self.owned_browsers(&owner).await;
        let mut sessions = Vec::new();
        for page in pages {
            if page.driver.is_closed() {
                continue;
            }
            let meta = page.activity.lock().await;
            sessions.push(serde_json::json!({
                "session_id": page.id, "opener_session_id": page.opener_id,
                "title": meta.title, "url": meta.url, "attached": true,
            }));
        }
        let running = self.browser_state.local.running().await?;
        let summary = format!(
            "{} browser pages · {} previews",
            sessions.len(),
            previews.len()
        );
        let data = serde_json::json!({"browser_running": running, "sessions": sessions, "previews": previews});
        Ok(ToolInvokeOutput::from_parts(
            "List browser pages",
            summary,
            data.to_string(),
            Some(data),
            Default::default(),
            Vec::new(),
        ))
    }

    pub(super) async fn close_browser(
        &self,
        context: &ToolInvokeContext<'_>,
        id: &str,
    ) -> SdkResult<ToolInvokeOutput> {
        let _control =
            handoff::agent_access(self.browser_state.control(context.session_id)).await?;
        let page = self
            .browser_state
            .owned_page(&browser_owner(context)?, id)
            .await?;
        self.browser_state
            .close(
                &page,
                BackgroundActivityStatus::Stopped,
                "Closed by owning session.",
                self.state()?.host.as_ref(),
            )
            .await?;
        Ok(browser_output(
            "Close browser page",
            "Browser page closed".into(),
            serde_json::json!({"session_id": id, "closed": true}),
        ))
    }

    async fn owned_browsers(&self, owner: &BrowserOwner) -> Vec<Arc<BrowserPage>> {
        self.browser_state
            .pages
            .lock()
            .await
            .values()
            .filter(|page| &page.owner == owner)
            .cloned()
            .collect()
    }

    pub(super) async fn close_owned_browsers(
        &self,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        let _control =
            handoff::agent_access(self.browser_state.control(context.session_id)).await?;
        let owner = browser_owner(context)?;
        let key = (owner.workspace.clone(), owner.session_id);
        let mut root = self.browser_state.root.lock().await;
        let host = self.state()?.host.as_ref();
        let mut closed = Vec::new();
        let mut failures = Vec::new();
        for page in self.owned_browsers(&owner).await {
            match self
                .browser_state
                .close(
                    &page,
                    BackgroundActivityStatus::Stopped,
                    "Closed by owning session.",
                    host,
                )
                .await
            {
                Ok(()) => closed.push(page.id.clone()),
                Err(error) => {
                    failures.push(format!("{}: {}", page.id, error.failure.user.fallback))
                }
            }
        }
        if let Some(root) = root.as_mut()
            && let Some(id) = root.contexts.get(&key)
            && failures.is_empty()
        {
            if root.driver.is_closed() {
                root.contexts.remove(&key);
            } else {
                match root.driver.dispose_context(id).await {
                    Ok(_) => {
                        root.contexts.remove(&key);
                    }
                    Err(error) => failures.push(error.failure.user.fallback.clone()),
                }
            }
        }
        Ok(browser_output(
            "Close owned browser pages",
            format!("{} closed · {} failures", closed.len(), failures.len()),
            serde_json::json!({"closed_sessions": closed, "failures": failures}),
        ))
    }

    pub(super) async fn inspect_browser(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserSnapshotInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let access = self.browser_access(context, &input.session_id).await?;
        let snapshot = access.page.snapshot(&input.options()).await?;
        Ok(snapshot.output(
            "Browser snapshot",
            serde_json::json!({"session_id": input.session_id, "snapshot": snapshot}),
        ))
    }

    pub(super) async fn run_browser_command(
        &self,
        access: &BrowserAccess,
        action: &str,
        command: serde_json::Value,
        timeout_ms: u64,
        snapshot: bool,
    ) -> SdkResult<ToolInvokeOutput> {
        let page = &access.page;
        let frame_id = command["frameId"].as_str().map(str::to_owned);
        let result = match page
            .driver
            .action_and_settle(command, Duration::from_millis(timeout_ms))
            .await
        {
            Ok(result) => result,
            Err(error)
                if action != "wait"
                    && matches!(
                        error.kind,
                        PluginErrorKind::Timeout | PluginErrorKind::Disconnected
                    ) =>
            {
                let next_step = if page.driver.is_closed() {
                    "Check browser.list before opening another page."
                } else {
                    "Take a browser.snapshot and check whether the action took effect."
                };
                return Ok(browser_output(
                    "Browser action",
                    format!("Browser {action} outcome is unknown. {next_step}"),
                    serde_json::json!({"session_id": page.id, "action": {"kind": action, "status": "outcome_unknown"}, "next_step": next_step}),
                ));
            }
            Err(error) => return Err(error),
        };
        let mut payload = serde_json::json!({"session_id": page.id, "action": {"kind": action, "status": "completed", "result": result}});
        let opened = page.driver.settle_popups().await;
        if !opened.is_empty() {
            payload["opened_sessions"] = serde_json::json!(opened);
        }
        page.verify_preview_lease().await?;
        if result["dialogOpened"] == true {
            return Ok(browser_output(
                "Browser action",
                "A JavaScript dialog is open. Resolve it with browser.action using kind dialog."
                    .into(),
                payload,
            ));
        }
        let observation = if snapshot {
            page.snapshot(&SnapshotOptions {
                frame_id,
                ..SnapshotOptions::default()
            })
            .await
            .map(Some)
        } else {
            page.verify_committed_url().await.map(|()| None)
        };
        match observation {
            Ok(Some(snapshot)) => {
                page.publish_snapshot(&snapshot, self.state()?.host.as_ref())
                    .await;
                payload["snapshot"] = serde_json::json!(snapshot);
                return Ok(snapshot.output("Browser action", payload));
            }
            Ok(None) => {}
            Err(error) => {
                let message = error.failure.user.fallback;
                let summary = format!("Browser {action} completed. {message}");
                payload["observation"] = serde_json::json!({
                    "status": "failed", "kind": error.kind, "message": message,
                });
                return Ok(browser_output("Browser action", summary, payload));
            }
        }
        Ok(browser_output(
            "Browser action",
            format!("Completed browser {action}"),
            payload,
        ))
    }
}

impl BrowserRoot {
    async fn context_for(&mut self, owner: &BrowserOwner) -> SdkResult<String> {
        let key = (owner.workspace.clone(), owner.session_id);
        if let Some(context) = self.contexts.get(&key) {
            return Ok(context.clone());
        }
        if self.contexts.len() >= 64 {
            return Err(PluginError::invalid_params(
                "Browser context quota reached. Close an owned context with browser.shutdown.",
            ));
        }
        let id = self.driver.create_context().await?;
        self.contexts.insert(key, id.clone());
        Ok(id)
    }
}

impl BrowserSessions {
    fn observe(
        self: &Arc<Self>,
        page: &Arc<BrowserPage>,
        mut events: mpsc::Receiver<CdpEvent>,
        host: Arc<dyn HostClient>,
    ) {
        let state = Arc::downgrade(self);
        let page = Arc::downgrade(page);
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                let Some(page) = page.upgrade() else { return };
                page.record_navigation(event, host.as_ref()).await;
            }
            if let (Some(page), Some(state)) = (page.upgrade(), state.upgrade())
                && let Err(error) = state
                    .close(
                        &page,
                        BackgroundActivityStatus::Failed,
                        "The browser page disconnected.",
                        host.as_ref(),
                    )
                    .await
                && error.kind != PluginErrorKind::Disconnected
            {
                log_secondary_plugin_failure("retire disconnected browser page", &error);
            }
        });
    }
}

impl BrowserPage {
    async fn record_navigation(&self, event: CdpEvent, host: &dyn HostClient) {
        let Some(url) = event
            .params
            .pointer("/frame/url")
            .and_then(serde_json::Value::as_str)
            .and_then(|raw| url::Url::parse(raw).ok())
            .filter(|url| matches!(url.scheme(), "http" | "https"))
        else {
            return;
        };
        let mut meta = self.activity.lock().await;
        if meta.url == url.as_str() {
            return;
        }
        meta.url = url.to_string();
        meta.title = format!("Browser · {}", browser_title_target(&url));
        drop(meta);
        self.publish(BackgroundActivityStatus::Running, None, host)
            .await;
    }
}

pub(super) fn navigation_command(url: &url::Url, timeout: u64) -> serde_json::Value {
    serde_json::json!({"action": "navigate", "url": url.as_str(), "waitUntil": "domcontentloaded", "timeout": timeout})
}

#[derive(Default)]
struct SnapshotOptions {
    selector: Option<String>,
    frame_id: Option<String>,
    interactive: bool,
    max_depth: Option<u8>,
}

#[derive(Serialize)]
struct BrowserSnapshot {
    title: String,
    url: String,
    tree: String,
    truncated: bool,
}

impl BrowserSnapshot {
    fn output(&self, title: &str, mut payload: serde_json::Value) -> ToolInvokeOutput {
        payload["takeover_hint"] = serde_json::json!(
            "If the page shows human verification, sign-in, or another step the user must perform, read tools_help for browser.request_takeover and invoke it with this page's session_id and a short reason before ending the task. Wait for handback, then take a fresh snapshot of the returned page and continue."
        );
        let page_title = self.title.trim();
        let summary = if page_title.is_empty() {
            self.url.as_str()
        } else {
            page_title
        };
        ToolInvokeOutput::from_parts(
            title,
            browser_text_prefix(summary, 512),
            format!(
                "{}\n{}\n\n{}{}",
                browser_text_prefix(summary, 512),
                browser_text_prefix(&self.url, 1024),
                self.tree,
                opened_pages_text(&payload),
            ),
            Some(payload),
            Default::default(),
            Vec::new(),
        )
    }
}

pub(super) fn browser_output(
    title: &str,
    summary: String,
    payload: serde_json::Value,
) -> ToolInvokeOutput {
    let output = format!("{summary}{}", opened_pages_text(&payload));
    ToolInvokeOutput::from_parts(
        title,
        &summary,
        output,
        Some(payload),
        Default::default(),
        Vec::new(),
    )
}

fn opened_pages_text(payload: &serde_json::Value) -> String {
    let Some(opened) = payload["opened_sessions"].as_array() else {
        return String::new();
    };
    let ids: Vec<_> = opened
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    format!(
        "\n\nNew browser pages: {}. Use browser.list for their URLs.",
        ids.join(", ")
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector", "frame_id"),
    non_empty("session_id"),
    non_empty_if_present("selector", "frame_id")
)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserSnapshotInput {
    session_id: String,
    /// Scope the accessibility tree to a CSS selector in the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    selector: Option<String>,
    /// Frame ID from browser.frames. Defaults to the main document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    /// Return interactive controls and omit static page content.
    #[serde(default)]
    interactive: bool,
    /// Limit how deep the accessibility tree descends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 30))]
    max_depth: Option<u8>,
}

impl BrowserSnapshotInput {
    fn options(&self) -> SnapshotOptions {
        SnapshotOptions {
            selector: self.selector.clone(),
            frame_id: self.frame_id.clone(),
            interactive: self.interactive,
            max_depth: self.max_depth,
        }
    }
}

fn browser_activity(
    id: String,
    title: String,
    description: String,
    status: BackgroundActivityStatus,
    started_at_ms: Option<i64>,
    finished_at_ms: Option<i64>,
    message: Option<String>,
) -> BackgroundActivity {
    let now = chrono::Utc::now().timestamp_millis();
    BackgroundActivity {
        id,
        kind: BackgroundActivityKind::Browser,
        status,
        title,
        description,
        command: None,
        workdir: None,
        session_id: None,
        parent_session_id: None,
        operation_id: None,
        source_part_id: None,
        created_at_ms: started_at_ms.unwrap_or(now),
        started_at_ms: started_at_ms.unwrap_or(now),
        finished_at_ms,
        next_event_at_ms: None,
        exit_code: None,
        message,
        failure: None,
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        cancellable: status.is_active(),
        dismissible: status.is_terminal(),
    }
}

/// Human-facing title segment for a URL: host plus the non-root path.
fn browser_title_target(url: &url::Url) -> String {
    let mut title = url.host_str().unwrap_or(url.as_str()).to_owned();
    let path = url.path().trim_end_matches('/');
    if !path.is_empty() {
        title.push_str(path);
    }
    title
}

pub(super) fn browser_text_prefix(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub(super) const fn default_return_snapshot() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "frame_id"),
    non_empty("session_id"),
    non_empty_if_present("frame_id")
)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserActionInput {
    session_id: String,
    action: BrowserAction,
    /// Frame ID from browser.frames for an element action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    frame_id: Option<String>,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
    /// Return a fresh accessibility snapshot after the action.
    #[serde(default = "default_return_snapshot")]
    return_snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserAction {
    Navigate {
        /// Absolute HTTP(S) URL. For a managed preview, use its own origin.
        url: String,
    },
    Back,
    Forward,
    Reload,
    Press {
        /// A key or chord in the focused control, such as Enter or Control+a.
        #[schemars(length(min = 1, max = 64))]
        key: String,
    },
    Scroll {
        direction: ScrollDirection,
        /// Pixel distance. Chrome scrolls by 300 pixels when omitted.
        #[schemars(range(min = 1, max = 100000))]
        amount: Option<u32>,
        /// Optional @eN ref or CSS selector for a scrollable element.
        #[schemars(length(min = 1, max = 4096))]
        selector: Option<String>,
    },
    Select {
        /// The select element, identified by @eN or a CSS selector.
        #[schemars(length(min = 1, max = 4096))]
        selector: String,
        /// The option values or visible labels to select.
        #[schemars(length(min = 1, max = 32))]
        values: Vec<String>,
    },
    SetChecked {
        /// The checkbox or radio control, identified by @eN or a CSS selector.
        #[schemars(length(min = 1, max = 4096))]
        selector: String,
        checked: bool,
    },
    Hover {
        #[schemars(length(min = 1, max = 4096))]
        selector: String,
    },
    DoubleClick {
        #[schemars(length(min = 1, max = 4096))]
        selector: String,
    },
    Focus {
        #[schemars(length(min = 1, max = 4096))]
        selector: String,
    },
    ScrollIntoView {
        #[schemars(length(min = 1, max = 4096))]
        selector: String,
    },
    Drag {
        /// Source element, as a selector or snapshot ref.
        #[schemars(length(min = 1, max = 4096))]
        source: String,
        /// Destination element, as a selector or snapshot ref.
        #[schemars(length(min = 1, max = 4096))]
        target: String,
    },
    Dialog {
        /// Accept the open JavaScript dialog when true, dismiss it when false.
        accept: bool,
        /// Value to submit to a JavaScript prompt.
        #[schemars(length(max = 4096))]
        prompt_text: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

impl BrowserPlugin {
    pub(super) async fn control_browser(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserActionInput,
    ) -> SdkResult<ToolInvokeOutput> {
        use serde_json::json;

        if input.frame_id.is_some()
            && matches!(
                input.action,
                BrowserAction::Navigate { .. }
                    | BrowserAction::Back
                    | BrowserAction::Forward
                    | BrowserAction::Reload
                    | BrowserAction::Dialog { .. }
            )
        {
            return Err(PluginError::invalid_params(
                "This action applies to the whole page. Omit frame_id.",
            ));
        }
        let access = self.browser_access(context, &input.session_id).await?;
        let (kind, mut command) = match &input.action {
            BrowserAction::Navigate { url } => {
                let url = prepare_fetch_url(url.trim()).map_err(PluginError::invalid_params)?;
                self.validate_browser_destination(access.page.preview.as_ref(), &url)
                    .await?;
                ("navigate", navigation_command(&url, input.timeout_ms))
            }
            BrowserAction::Back => ("back", json!({"action": "back"})),
            BrowserAction::Forward => ("forward", json!({"action": "forward"})),
            BrowserAction::Reload => ("reload", json!({"action": "reload"})),
            BrowserAction::Press { key } => {
                ("press", json!({"action": "press", "key": key.trim()}))
            }
            BrowserAction::Scroll {
                direction,
                amount,
                selector,
            } => (
                "scroll",
                json!({
                    "action": "scroll", "direction": direction, "amount": amount,
                    "selector": selector.as_deref().map(str::trim).or(input.frame_id.as_ref().map(|_| ":root")),
                }),
            ),
            BrowserAction::Select { selector, values } => (
                "select",
                json!({
                    "action": "select", "selector": selector.trim(), "values": values,
                }),
            ),
            BrowserAction::SetChecked { selector, checked } => (
                "set_checked",
                json!({
                    "action": if *checked { "check" } else { "uncheck" }, "selector": selector.trim(),
                }),
            ),
            BrowserAction::Dialog {
                accept,
                prompt_text,
            } => (
                "dialog",
                json!({
                    "action": "dialog", "accept": accept, "promptText": prompt_text,
                }),
            ),
            BrowserAction::Hover { selector } => (
                "hover",
                json!({"action": "hover", "selector": selector.trim()}),
            ),
            BrowserAction::DoubleClick { selector } => (
                "double_click",
                json!({"action": "double_click", "selector": selector.trim()}),
            ),
            BrowserAction::Focus { selector } => (
                "focus",
                json!({"action": "focus", "selector": selector.trim()}),
            ),
            BrowserAction::ScrollIntoView { selector } => (
                "scroll_into_view",
                json!({"action": "scroll_into_view", "selector": selector.trim()}),
            ),
            BrowserAction::Drag { source, target } => (
                "drag",
                json!({"action": "drag", "source": source.trim(), "target": target.trim()}),
            ),
        };
        command["frameId"] = json!(input.frame_id);
        self.run_browser_command(
            &access,
            kind,
            command,
            input.timeout_ms,
            input.return_snapshot,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn browser_snapshot_accepts_default_and_scoped_inputs() {
        let default = BrowserSnapshotInput::parse_input(json!({"session_id": "page"})).unwrap();
        assert!(default.options().selector.is_none());
        let scoped = BrowserSnapshotInput::parse_input(json!({
            "session_id": "page", "selector": " main form ", "interactive": true, "max_depth": 4,
        }))
        .unwrap();
        assert_eq!(scoped.options().selector.as_deref(), Some("main form"));
        assert!(
            BrowserSnapshotInput::parse_input(json!({"session_id": "page", "selector": " "}))
                .is_err()
        );
    }
}
