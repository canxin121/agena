//! Agena's request authorization and event consumers for the native CDP engine.

mod diagnostics;
mod navigation;

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex as SyncMutex, Weak,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

pub(crate) use agena_agent_browser::agena::CdpEvent;
use agena_agent_browser::agena::{CdpClient, PageEngine, connect_cdp, is_session_detached_error};
use agena_plugin_host::sdk::{Result as SdkResult, host_api, permission_wait};
use agena_plugin_host::{PluginError, sdk::PluginErrorKind};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, RwLock, Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use diagnostics::Diagnostics;
pub(crate) use diagnostics::{MAX_BODY_BYTES, ResponseBody, console_text};

pub(crate) enum DomFormat {
    Text,
    Html,
}

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PENDING_REQUESTS: usize = 512;
pub(crate) const CHROME_ERROR_DOCUMENT: &str = "chrome-error://chromewebdata/";
const PAGE_EVENTS: &[&str] = &[
    "Fetch.requestPaused",
    "Page.frameNavigated",
    "Page.navigatedWithinDocument",
    "Page.windowOpen",
    "Page.javascriptDialogOpening",
    "Page.javascriptDialogClosed",
    "Runtime.executionContextCreated",
    "Runtime.executionContextDestroyed",
    "Runtime.executionContextsCleared",
    "Runtime.consoleAPICalled",
    "Runtime.exceptionThrown",
    "Log.entryAdded",
    "Network.requestWillBeSent",
    "Network.loadingFinished",
    "Network.loadingFailed",
    "Network.dataReceived",
    "Network.responseReceived",
    "Target.attachedToTarget",
    "Target.detachedFromTarget",
];
const DOWNLOAD_EVENTS: &[&str] = &["Browser.downloadWillBegin", "Browser.downloadProgress"];

pub(crate) type BrowserRequestPolicy =
    Arc<dyn Fn(String, bool) -> Pin<Box<dyn Future<Output = SdkResult<()>> + Send>> + Send + Sync>;

pub(crate) enum PageEvents {
    Activity {
        sender: mpsc::Sender<CdpEvent>,
        capture_console: bool,
        allow_popups: bool,
    },
    Network(mpsc::Sender<CdpEvent>),
}

impl PageEvents {
    fn retains_diagnostics(&self) -> bool {
        matches!(self, Self::Activity { .. })
    }

    fn forward(&self, event: CdpEvent) -> Result<(), String> {
        let network_event = event.method.starts_with("Network.");
        match self {
            Self::Activity { sender, .. }
                if matches!(
                    event.method.as_str(),
                    "Page.frameNavigated"
                        | "Page.navigatedWithinDocument"
                        | "Runtime.consoleAPICalled"
                        | "Runtime.exceptionThrown"
                        | "Log.entryAdded"
                ) =>
            {
                let _ = sender.try_send(event);
            }
            Self::Network(sender) if network_event => {
                if let Err(mpsc::error::TrySendError::Full(_)) = sender.try_send(event) {
                    return Err("Browser network event buffer overflow".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone)]
enum RequestAuthority {
    Agent {
        context: host_api::HostCallbackContext,
        policy: BrowserRequestPolicy,
    },
    Human(BrowserRequestPolicy),
}

struct RequestScope {
    authority: RequestAuthority,
    denied_document: SyncMutex<Option<PluginError>>,
    finished: CancellationToken,
    popups: SyncMutex<Popups>,
    popup_changed: Notify,
}

#[derive(Default)]
struct Popups {
    announced: usize,
    started: usize,
    pending: usize,
    opened: Vec<String>,
}

impl RequestScope {
    fn popups(&self) -> std::sync::MutexGuard<'_, Popups> {
        self.popups
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

pub(crate) struct PopupScope(Arc<RequestScope>);

impl PopupScope {
    pub fn is_active(&self) -> bool {
        !self.0.finished.is_cancelled()
    }

    pub fn is_human(&self) -> bool {
        matches!(self.0.authority, RequestAuthority::Human(_))
    }

    pub fn opened(&self, id: String) {
        self.0.popups().opened.push(id);
    }

    pub fn callback_context(&self) -> host_api::HostCallbackContext {
        match &self.0.authority {
            RequestAuthority::Agent { context, .. } => context.clone(),
            RequestAuthority::Human(_) => host_api::HostCallbackContext::default(),
        }
    }
}

impl Drop for PopupScope {
    fn drop(&mut self) {
        self.0.popups().pending -= 1;
        self.0.popup_changed.notify_one();
    }
}

struct PageEventsState {
    main_frame: String,
    frame_sessions: RwLock<HashMap<String, String>>,
    frame_contexts: RwLock<HashMap<String, FrameContext>>,
    document_revision: AtomicU64,
    authority: SyncMutex<Option<Arc<RequestScope>>>,
    inherited_authority: Option<Arc<RequestScope>>,
    diagnostics: SyncMutex<Diagnostics>,
    private: std::sync::atomic::AtomicBool,
    saw_document: std::sync::atomic::AtomicBool,
    dialog: SyncMutex<Option<agena_plugin_sdk::activity::browser::BrowserDialog>>,
}

#[derive(Clone)]
struct FrameContext {
    session: String,
    unique_id: String,
    id: i64,
}

impl PageEventsState {
    fn new(target: &str, popup: Option<&PopupScope>) -> Self {
        Self {
            main_frame: target.to_owned(),
            frame_sessions: RwLock::default(),
            frame_contexts: RwLock::default(),
            document_revision: AtomicU64::new(0),
            authority: SyncMutex::new(None),
            inherited_authority: popup.map(|popup| {
                Arc::new(RequestScope {
                    authority: popup.0.authority.clone(),
                    denied_document: SyncMutex::new(None),
                    finished: popup.0.finished.child_token(),
                    popups: SyncMutex::default(),
                    popup_changed: Notify::new(),
                })
            }),
            diagnostics: SyncMutex::new(Diagnostics::new(target)),
            private: std::sync::atomic::AtomicBool::new(
                popup.is_some_and(|popup| matches!(popup.0.authority, RequestAuthority::Human(_))),
            ),
            saw_document: std::sync::atomic::AtomicBool::new(false),
            dialog: SyncMutex::default(),
        }
    }

    fn diagnostics(&self) -> std::sync::MutexGuard<'_, Diagnostics> {
        self.diagnostics
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn active_scope(&self) -> Option<Arc<RequestScope>> {
        self.authority
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .or_else(|| self.inherited_authority.clone())
            .filter(|scope| !scope.finished.is_cancelled())
    }
}

/// A page transaction (e.g. navigate and take the resulting snapshot) shares
/// one callback identity. Requests arriving between tool invocations have no
/// authority, and queued requests keep the identity that first observed them.
pub(crate) struct BrowserAuthorization {
    events: Arc<PageEventsState>,
    scope: Arc<RequestScope>,
}

impl Drop for BrowserAuthorization {
    fn drop(&mut self) {
        self.cancel_requests();
        self.events
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
}

impl BrowserAuthorization {
    pub(crate) fn cancel_requests(&self) {
        self.scope.finished.cancel();
    }
}

struct Page {
    target: String,
    session: String,
    engine: Mutex<PageEngine>,
    events: Arc<PageEventsState>,
}

struct Connection {
    client: Arc<CdpClient>,
    closed: CancellationToken,
    page: Option<Page>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.closed.cancel();
        let client = self.client.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { client.close().await });
        }
    }
}

#[derive(Clone)]
pub(crate) struct BrowserDriver(Arc<Connection>);

pub(crate) struct BrowserTargetWatcher {
    root: Weak<Connection>,
    closed: CancellationToken,
    events: mpsc::UnboundedReceiver<CdpEvent>,
}

impl BrowserTargetWatcher {
    pub async fn next(&mut self) -> Option<(BrowserDriver, CdpEvent)> {
        tokio::select! {
            _ = self.closed.cancelled() => None,
            event = self.events.recv() => Some((BrowserDriver(self.root.upgrade()?), event?)),
        }
    }
}

impl BrowserDriver {
    pub(crate) async fn create_context(&self) -> SdkResult<String> {
        let context = self
            .command(
                "Target.createBrowserContext",
                json!({
                    "disposeOnDetach": true,
                }),
            )
            .await?;
        let id = context["browserContextId"]
            .as_str()
            .ok_or_else(|| PluginError::internal("Chrome returned no browser context ID"))?;
        if let Err(error) = self
            .command(
                "Browser.setDownloadBehavior",
                json!({
                    "behavior": "deny", "browserContextId": id,
                }),
            )
            .await
        {
            let _ = self.dispose_context(id).await;
            return Err(error);
        }
        Ok(id.to_owned())
    }

    pub(crate) async fn dispose_context(&self, context: &str) -> SdkResult<()> {
        self.command(
            "Target.disposeBrowserContext",
            json!({"browserContextId": context}),
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn create_blank_target(&self, context: &str) -> SdkResult<String> {
        let target = self
            .command(
                "Target.createTarget",
                json!({
                    "url": "about:blank", "browserContextId": context,
                }),
            )
            .await?;
        target["targetId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| PluginError::internal("Chrome returned no page ID"))
    }

    pub async fn connect_root(
        endpoint: &str,
        download_events: Option<mpsc::Sender<CdpEvent>>,
    ) -> SdkResult<Self> {
        let client = connect(endpoint).await?;
        let closed = CancellationToken::new();
        if let Some(sender) = download_events {
            let mut events = client.subscribe_methods(DOWNLOAD_EVENTS);
            let client = client.clone();
            let stop = closed.clone();
            tokio::spawn(async move {
                let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = heartbeat.tick() => {
                            if !client.is_connected() { break; }
                        }
                        event = events.recv() => {
                            let Some(event) = event else { break };
                            if sender.try_send(event).is_err() { break; }
                        }
                    }
                }
                stop.cancel();
                client.close().await;
            });
        }
        Ok(Self(Arc::new(Connection {
            client,
            closed,
            page: None,
        })))
    }

    pub async fn connect_page(
        endpoint: &str,
        target: &str,
        observer: PageEvents,
    ) -> SdkResult<Self> {
        Self::connect_page_with_popup(endpoint, target, observer, None).await
    }

    pub async fn connect_page_with_popup(
        endpoint: &str,
        target: &str,
        observer: PageEvents,
        popup: Option<&PopupScope>,
    ) -> SdkResult<Self> {
        let client = connect(endpoint).await?;
        let events = client.subscribe_methods(PAGE_EVENTS);
        let mut connection = Connection {
            client: client.clone(),
            closed: CancellationToken::new(),
            page: None,
        };
        let attached = async {
            let attached = client
                .send_command(
                    "Target.attachToTarget",
                    Some(json!({"targetId": target, "flatten": true})),
                    None,
                )
                .await?;
            let session = attached["sessionId"]
                .as_str()
                .ok_or("Chrome returned no page session")?;
            if popup.is_some() {
                // A new noopener tab has no initialized renderer while Chrome
                // holds it at the debugger barrier. Fetch and Target run in
                // the browser process and can protect its initial navigation.
                install_page_policy(&client, session).await?;
            } else {
                prepare_page(&client, session, observer.retains_diagnostics()).await?;
            }
            Ok::<_, String>(session.to_owned())
        };
        let session = command_result(permission_wait::timeout(CONNECT_TIMEOUT, attached).await)?;
        let engine = PageEngine::new(
            client.clone(),
            endpoint.to_owned(),
            target.to_owned(),
            session.clone(),
        );
        let state = Arc::new(PageEventsState::new(target, popup));
        let event_state = state.clone();
        let event_session = session.clone();
        let closed = connection.closed.clone();
        tokio::spawn(async move {
            if let Err(error) = run_page_events(
                &client,
                events,
                &event_session,
                observer,
                event_state,
                &closed,
            )
            .await
            {
                tracing::debug!(%error, "browser page CDP event handler stopped");
            }
            closed.cancel();
            client.close().await;
        });
        connection.page = Some(Page {
            target: target.to_owned(),
            session,
            engine: Mutex::new(engine),
            events: state,
        });
        Ok(Self(Arc::new(connection)))
    }

    pub async fn watch_new_pages(&self) -> SdkResult<BrowserTargetWatcher> {
        let events = self
            .0
            .client
            .subscribe_methods(&["Target.attachedToTarget"]);
        self.command(
            "Target.setAutoAttach",
            json!({
                "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true,
                "filter": [{"type": "page"}, {"exclude": true}],
            }),
        )
        .await?;
        Ok(BrowserTargetWatcher {
            root: Arc::downgrade(&self.0),
            closed: self.0.closed.clone(),
            events,
        })
    }

    pub async fn resume_new_page(&self, session: &str) -> SdkResult<()> {
        self.command_in_session(
            Some(session),
            "Runtime.runIfWaitingForDebugger",
            json!({}),
            Duration::from_secs(5),
        )
        .await
        .map(|_| ())
    }

    pub async fn enable_popup_page(&self) -> SdkResult<()> {
        let page = self.page()?;
        command_result(
            permission_wait::timeout(
                CONNECT_TIMEOUT,
                enable_page_domains(&self.0.client, &page.session, true),
            )
            .await,
        )
    }

    pub fn popup_scope(&self) -> Option<PopupScope> {
        let scope = self.0.page.as_ref()?.events.active_scope()?;
        {
            let mut popups = scope.popups();
            if popups.started >= 8 {
                return None;
            }
            popups.started += 1;
            popups.pending += 1;
        }
        scope.popup_changed.notify_one();
        Some(PopupScope(scope))
    }

    pub async fn settle_popups(&self) -> Vec<String> {
        let Some(scope) = self
            .0
            .page
            .as_ref()
            .and_then(|page| page.events.active_scope())
        else {
            return Vec::new();
        };
        let _ = permission_wait::timeout(Duration::from_secs(6), async {
            loop {
                let changed = scope.popup_changed.notified();
                let pending = {
                    let popups = scope.popups();
                    if popups.pending == 0 && popups.started >= popups.announced.min(8) {
                        break;
                    }
                    popups.pending > 0
                };
                if pending {
                    changed.await;
                } else if tokio::time::timeout(Duration::from_millis(350), changed)
                    .await
                    .is_err()
                {
                    // A windowOpen announcement can refer to a reused window
                    // or a popup Chrome chose not to open.
                    break;
                }
            }
        })
        .await;
        scope.popups().opened.clone()
    }

    pub fn saw_document_request(&self) -> SdkResult<bool> {
        Ok(self.page()?.events.saw_document.load(Ordering::Acquire))
    }

    /// Keep observation of a page that has accepted human input out of the
    /// activity and diagnostic history, including events delivered late by CDP.
    pub fn make_private(&self) -> SdkResult<()> {
        let page = self.page()?;
        let mut diagnostics = page.events.diagnostics();
        page.events.private.store(true, Ordering::Release);
        *diagnostics = Diagnostics::new(&page.target);
        page.events
            .document_revision
            .fetch_add(1, Ordering::Release);
        Ok(())
    }

    pub fn dialog(&self) -> SdkResult<Option<agena_plugin_sdk::activity::browser::BrowserDialog>> {
        Ok(self
            .page()?
            .events
            .dialog
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone())
    }

    /// An operator observation must not consume a document rejection recorded
    /// for an in-flight agent call.
    pub async fn viewport_command(&self, method: &str, params: Value) -> SdkResult<Value> {
        self.ensure_connected()?;
        let page = self.page()?;
        command_result(
            tokio::time::timeout(
                Duration::from_secs(3),
                self.0
                    .client
                    .send_command(method, Some(params), Some(&page.session)),
            )
            .await,
        )
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.is_cancelled() || !self.0.client.is_connected()
    }

    pub fn disconnect(&self) {
        self.0.closed.cancel();
    }

    pub fn authorize(&self, policy: BrowserRequestPolicy) -> SdkResult<BrowserAuthorization> {
        let context = host_api::current_host_callback_context().ok_or_else(|| {
            PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "Browser action needs an active tool call",
            )
        })?;
        self.authorize_requests(RequestAuthority::Agent { context, policy })
    }

    pub fn authorize_human(&self, policy: BrowserRequestPolicy) -> SdkResult<BrowserAuthorization> {
        self.authorize_requests(RequestAuthority::Human(policy))
    }

    fn authorize_requests(
        &self,
        request_authority: RequestAuthority,
    ) -> SdkResult<BrowserAuthorization> {
        let page = self.page()?;
        self.ensure_connected()?;
        let mut authority = page
            .events
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if authority.is_some() {
            return Err(PluginError::internal(
                "The browser page already has an active tool call",
            ));
        }
        let scope = Arc::new(RequestScope {
            authority: request_authority,
            denied_document: SyncMutex::new(None),
            finished: CancellationToken::new(),
            popups: SyncMutex::default(),
            popup_changed: Notify::new(),
        });
        *authority = Some(scope.clone());
        Ok(BrowserAuthorization {
            events: page.events.clone(),
            scope,
        })
    }

    pub async fn command(&self, method: &str, params: Value) -> SdkResult<Value> {
        self.command_with_timeout(method, params, COMMAND_TIMEOUT)
            .await
    }

    pub async fn command_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> SdkResult<Value> {
        self.command_in_session(
            self.0.page.as_ref().map(|page| page.session.as_str()),
            method,
            params,
            timeout,
        )
        .await
    }

    async fn command_in_session(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> SdkResult<Value> {
        self.call(
            timeout,
            self.0.client.send_command(method, Some(params), session),
        )
        .await
    }

    pub async fn evaluate(&self, expression: &str) -> SdkResult<Value> {
        let value = self
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": expression, "returnByValue": true, "awaitPromise": true,
                }),
            )
            .await?;
        if let Some(exception) = value.get("exceptionDetails") {
            return Err(PluginError::internal(format!(
                "Browser evaluation failed: {exception}"
            )));
        }
        Ok(value
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub async fn evaluate_script(
        &self,
        expression: &str,
        frame_id: Option<&str>,
        timeout: Duration,
    ) -> SdkResult<Value> {
        let page = self.page()?;
        let context = if frame_id.is_some() {
            self.frame_context(frame_id).await?
        } else {
            page.events
                .frame_contexts
                .read()
                .await
                .get(&page.target)
                .cloned()
        };
        let mut params = json!({
            "expression": expression, "returnByValue": true, "awaitPromise": true,
            "timeout": timeout.as_millis() as u64,
            "allowUnsafeEvalBlockedByCSP": false,
        });
        if let Some(context) = &context {
            params["uniqueContextId"] = json!(context.unique_id);
        }
        self.command_in_session(
            Some(
                context
                    .as_ref()
                    .map_or(page.session.as_str(), |frame| frame.session.as_str()),
            ),
            "Runtime.evaluate",
            params,
            timeout + Duration::from_millis(250),
        )
        .await
    }

    pub fn document_revision(&self) -> SdkResult<u64> {
        Ok(self
            .page()?
            .events
            .document_revision
            .load(Ordering::Acquire))
    }

    pub async fn context_id(&self, frame: Option<&str>) -> SdkResult<Option<String>> {
        let page = self.page()?;
        let contexts = page.events.frame_contexts.read().await;
        Ok(contexts
            .get(frame.unwrap_or(&page.target))
            .map(|context| context.unique_id.clone()))
    }

    pub async fn child_frame_ids(&self) -> SdkResult<Vec<String>> {
        let page = self.page()?;
        Ok(page
            .events
            .frame_contexts
            .read()
            .await
            .keys()
            .filter(|frame| *frame != &page.target)
            .cloned()
            .collect())
    }

    async fn frame_context(&self, frame_id: Option<&str>) -> SdkResult<Option<FrameContext>> {
        let Some(frame_id) = frame_id else {
            return Ok(None);
        };
        let page = self.page()?;
        let contexts = page.events.frame_contexts.read().await;
        contexts.get(frame_id).cloned().map(Some).ok_or_else(|| {
            PluginError::invalid_params(
                "Frame is unavailable in this browser page. Refresh the list with browser.frames.",
            )
        })
    }

    pub async fn frames(&self) -> SdkResult<Value> {
        let page = self.page()?;
        let tree = self.command("Page.getFrameTree", json!({})).await?;
        let root = tree
            .pointer("/frameTree/frame/id")
            .and_then(Value::as_str)
            .unwrap_or(&page.target);
        let mut frames = BTreeMap::new();
        collect_frames(&tree["frameTree"], None, root, &mut frames);
        let sessions = page.events.frame_sessions.read().await.clone();
        let mut detached = 0;
        for session in sessions.values().take(128) {
            match self
                .command_in_session(
                    Some(session),
                    "Page.getFrameTree",
                    json!({}),
                    COMMAND_TIMEOUT,
                )
                .await
            {
                Ok(tree) => collect_frames(&tree["frameTree"], None, root, &mut frames),
                Err(error) if is_session_detached_error(error.diagnostic_message()) => {
                    detached += 1
                }
                Err(error) => return Err(error),
            }
        }
        let contexts = page.events.frame_contexts.read().await;
        let truncated = frames.len() > 128 || sessions.len() > 128;
        let frames: Vec<_> = frames
            .into_iter()
            .take(128)
            .map(|(id, mut frame)| {
                frame["context_ready"] = json!(contexts.contains_key(&id));
                frame
            })
            .collect();
        Ok(
            json!({"frames": frames, "main_frame_id": root, "truncated": truncated, "detached": detached}),
        )
    }

    pub async fn document_response(
        &self,
    ) -> SdkResult<Result<(String, ResponseBody), &'static str>> {
        let page = self.page()?;
        let frame = self.main_frame().await?;
        let loader = frame["loaderId"].as_str().unwrap_or_default();
        let source = page.events.diagnostics().document(loader);
        let Some(source) = source else {
            return Ok(Err("not_captured"));
        };
        if !source.is_html() && source.entry_state() == "finished" {
            return Ok(Err("non_html"));
        }
        Ok(source
            .read(&self.0.client)
            .await
            .map(|body| (source.url().to_owned(), body)))
    }

    pub async fn main_frame(&self) -> SdkResult<Value> {
        let mut tree = self.command("Page.getFrameTree", json!({})).await?;
        let frame = tree["frameTree"]["frame"].take();
        if frame["url"].is_string() {
            Ok(frame)
        } else {
            Err(PluginError::internal(
                "Chrome did not report the main document",
            ))
        }
    }

    pub fn navigation_error(&self, frame: &Value) -> PluginError {
        let reason = self.0.page.as_ref().and_then(|page| {
            let url = frame["unreachableUrl"].as_str()?;
            page.events
                .diagnostics()
                .document_failure(url)
                .map(str::to_owned)
        });
        let message = match reason {
            Some(reason) => format!("Chrome could not load the page ({reason})."),
            None => "Chrome displayed a page load error.".into(),
        };
        PluginError::internal(format!(
            "{message} Use browser.diagnostics for network details or browser.action to navigate again."
        ))
    }

    pub fn network_requests(
        &self,
        since: u64,
        limit: usize,
        filter: Option<&str>,
    ) -> SdkResult<Value> {
        Ok(self
            .page()?
            .events
            .diagnostics()
            .requests(since, limit, filter))
    }

    pub fn diagnostics_logs(&self, since: u64, limit: usize) -> SdkResult<Value> {
        Ok(self.page()?.events.diagnostics().logs(since, limit))
    }

    pub async fn network_request(
        &self,
        id: u64,
        include_body: bool,
        max_chars: usize,
    ) -> SdkResult<Value> {
        let request = self.page()?.events.diagnostics().request(id);
        let Some(request) = request else {
            return Ok(json!({"request_id": id, "status": "not_retained"}));
        };
        let mut result = json!({"request": request.entry});
        if include_body {
            match request.read(&self.0.client).await {
                Ok(body) => {
                    let text: String = body.text.chars().take(max_chars).collect();
                    result["body"] = json!({
                        "status": "captured", "truncated": body.truncated || text.len() < body.text.len(),
                        "text": text,
                    });
                }
                Err(status) => result["body"] = json!({"status": status}),
            }
        }
        Ok(result)
    }

    pub(crate) async fn capture_dom(
        &self,
        format: DomFormat,
        selector: Option<&str>,
        frame_id: Option<&str>,
        byte_limit: usize,
        timeout: Duration,
    ) -> SdkResult<(String, ResponseBody)> {
        let format = match format {
            DomFormat::Text => "text",
            DomFormat::Html => "dom_html",
        };
        let captured = self
            .action(
                json!({
                    "action": "content", "format": format, "selector": selector,
                    "frameId": frame_id, "limit": byte_limit,
                }),
                timeout,
            )
            .await?;
        let url = captured["url"]
            .as_str()
            .ok_or_else(|| PluginError::internal("Chrome returned no DOM URL"))?;
        let content = captured["content"]
            .as_str()
            .ok_or_else(|| PluginError::internal("Chrome returned no DOM content"))?;
        let text = utf8_prefix(content, byte_limit);
        Ok((
            url.to_owned(),
            ResponseBody {
                text: text.to_owned(),
                truncated: text.len() < content.len() || captured["truncated"] == true,
            },
        ))
    }

    pub async fn action(&self, mut command: Value, timeout: Duration) -> SdkResult<Value> {
        let page = self.page()?;
        let frame = command["frameId"].as_str();
        if let Some(context) = self.frame_context(frame).await? {
            if frame == Some(page.target.as_str()) {
                command
                    .as_object_mut()
                    .expect("browser command is an object")
                    .remove("frameId");
            }
            command["uniqueContextId"] = json!(context.unique_id);
            command["executionSession"] = json!(context.session);
        }
        self.call(timeout, async {
            let mut engine = page.engine.lock().await;
            let frames = page.events.frame_sessions.read().await.clone();
            let revision = page.events.document_revision.load(Ordering::Acquire);
            Box::pin(engine.action(&command, frames, revision)).await
        })
        .await
    }

    pub async fn action_and_settle(&self, command: Value, timeout: Duration) -> SdkResult<Value> {
        if !matches!(
            command["action"].as_str(),
            Some(
                "navigate"
                    | "click"
                    | "double_click"
                    | "fill"
                    | "press"
                    | "select"
                    | "check"
                    | "uncheck"
                    | "dialog"
                    | "back"
                    | "forward"
                    | "reload"
            )
        ) {
            return self.action(command, timeout).await;
        }
        let page = self.page()?;
        let events = self.0.client.subscribe_methods(navigation::SETTLE_EVENTS);
        let deadline = permission_wait::Deadline::new(timeout);
        let result = self.action(command, deadline.remaining()).await?;
        if result["dialogOpened"] != true && self.dialog()?.is_none() {
            let budget = deadline.remaining().min(navigation::SETTLE_LIMIT);
            let _ = permission_wait::timeout(
                budget,
                navigation::settle_after_action(events, &page.session, &page.target),
            )
            .await;
            self.check_document_failure()?;
            self.ensure_connected()?;
        }
        Ok(result)
    }

    fn page(&self) -> SdkResult<&Page> {
        self.0
            .page
            .as_ref()
            .ok_or_else(|| PluginError::internal("Browser action needs an attached page"))
    }

    fn ensure_connected(&self) -> SdkResult<()> {
        if self.is_closed() {
            Err(disconnected())
        } else {
            Ok(())
        }
    }

    async fn call(
        &self,
        timeout: Duration,
        future: impl Future<Output = Result<Value, String>>,
    ) -> SdkResult<Value> {
        self.check_document_failure()?;
        self.ensure_connected()?;
        let result = tokio::select! {
            _ = self.0.closed.cancelled() => Err(disconnected()),
            result = permission_wait::timeout(timeout, future) => command_result(result),
        };
        self.check_document_failure()?;
        result
    }

    fn check_document_failure(&self) -> SdkResult<()> {
        let scope = self
            .0
            .page
            .as_ref()
            .and_then(|page| page.events.active_scope());
        if let Some(scope) = scope
            && let Some(error) = scope
                .denied_document
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
        {
            return Err(error);
        }
        Ok(())
    }
}

fn collect_frames(
    tree: &Value,
    parent: Option<&str>,
    main: &str,
    frames: &mut BTreeMap<String, Value>,
) {
    let frame = &tree["frame"];
    let Some(id) = frame["id"].as_str() else {
        return;
    };
    let parent = parent.or_else(|| frame["parentId"].as_str());
    let entry = frames.entry(id.to_owned()).or_insert_with(|| {
        json!({
            "frame_id": id, "main": id == main, "parent_frame_id": parent,
            "url": utf8_prefix(frame["url"].as_str().unwrap_or_default(), 2048),
            "name": utf8_prefix(frame["name"].as_str().unwrap_or_default(), 256),
        })
    });
    if entry["parent_frame_id"].is_null() && parent.is_some() {
        entry["parent_frame_id"] = json!(parent);
    }
    for child in tree["childFrames"].as_array().into_iter().flatten() {
        collect_frames(child, Some(id), main, frames);
    }
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    &text[..text.floor_char_boundary(max_bytes.min(text.len()))]
}

async fn connect(endpoint: &str) -> SdkResult<Arc<CdpClient>> {
    command_result(permission_wait::timeout(CONNECT_TIMEOUT, connect_cdp(endpoint)).await)
        .map(Arc::new)
}

fn disconnected() -> PluginError {
    PluginError::from_kind(
        PluginErrorKind::Disconnected,
        "The browser page is no longer connected. Open a new page with browser.open.",
    )
}

fn command_result<T, E>(result: Result<Result<T, String>, E>) -> SdkResult<T> {
    result
        .map_err(|_| PluginError::from_kind(PluginErrorKind::Timeout, "Browser command timed out"))?
        .map_err(|error| {
            if error.contains("timed out") {
                PluginError::from_kind(PluginErrorKind::Timeout, error)
            } else {
                PluginError::internal(error)
            }
        })
}

async fn prepare_page(client: &CdpClient, session: &str, diagnostics: bool) -> Result<(), String> {
    install_page_policy(client, session).await?;
    enable_page_domains(client, session, diagnostics).await
}

async fn enable_page_domains(
    client: &CdpClient,
    session: &str,
    diagnostics: bool,
) -> Result<(), String> {
    let (total_bytes, resource_bytes) = if diagnostics {
        (8 * MAX_BODY_BYTES, 2 * MAX_BODY_BYTES)
    } else {
        (65_536, 16_384)
    };
    for (method, params) in [
        (
            "Network.enable",
            json!({
                "maxTotalBufferSize": total_bytes, "maxResourceBufferSize": resource_bytes,
            }),
        ),
        ("Page.enable", json!({})),
        ("Runtime.enable", json!({})),
        ("Network.setBypassServiceWorker", json!({"bypass": true})),
        ("Network.setCacheDisabled", json!({"cacheDisabled": true})),
    ] {
        client
            .send_command(method, Some(params), Some(session))
            .await?;
    }
    if diagnostics {
        client
            .send_command("Log.enable", None, Some(session))
            .await?;
    }
    Ok(())
}

async fn install_page_policy(client: &CdpClient, session: &str) -> Result<(), String> {
    for (method, params) in [
        (
            "Fetch.enable",
            json!({"patterns": [{"urlPattern": "*", "requestStage": "Request"}]}),
        ),
        (
            "Target.setAutoAttach",
            json!({
                "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true,
                "filter": [{"type": "iframe"}, {"exclude": true}],
            }),
        ),
    ] {
        client
            .send_command(method, Some(params), Some(session))
            .await?;
    }
    Ok(())
}

async fn run_page_events(
    client: &Arc<CdpClient>,
    mut events: mpsc::UnboundedReceiver<CdpEvent>,
    session: &str,
    observer: PageEvents,
    state: Arc<PageEventsState>,
    closed: &CancellationToken,
) -> Result<(), String> {
    let mut requests = JoinSet::new();
    let mut body_reads = JoinSet::new();
    let slots = Arc::new(Semaphore::new(16));
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = closed.cancelled() => return Ok(()),
            _ = heartbeat.tick() => {
                if !client.is_connected() { return Ok(()); }
            }
            result = requests.join_next(), if !requests.is_empty() => {
                if let Some(result) = result { result.map_err(|error| error.to_string())??; }
            }
            result = body_reads.join_next(), if !body_reads.is_empty() => {
                if let Some(Ok((key, body))) = result {
                    let mut diagnostics = state.diagnostics();
                    if !state.private.load(Ordering::Acquire) {
                        diagnostics.save_body(&key, body);
                    }
                }
            }
            event = events.recv() => {
                let Some(event) = event else { return Ok(()); };
                let main = event.session_id.as_deref() == Some(session);
                if let PageEvents::Activity { capture_console, .. } = &observer
                    && let Some(key) = {
                        let mut diagnostics = state.diagnostics();
                        if state.private.load(Ordering::Acquire) {
                            None
                        } else {
                            diagnostics.record(&event, main, *capture_console)
                        }
                    }
                    && body_reads.len() < 4
                {
                    let client = client.clone();
                    body_reads.spawn(async move {
                        let body = diagnostics::read_body(&client, &key).await;
                        (key, body)
                    });
                }
                match event.method.as_str() {
                    "Fetch.requestPaused" => {
                        let main_document = main
                            && event.params["resourceType"] == "Document"
                            && event.params["frameId"].as_str() == Some(state.main_frame.as_str());
                        if main_document {
                            state.saw_document.store(true, Ordering::Release);
                        }
                        let scope = state.active_scope();
                        if requests.len() >= MAX_PENDING_REQUESTS {
                            deny_document(scope.as_deref(), main_document, PluginError::internal("Too many pending browser requests"));
                            decide_request(client, &event, false).await?;
                            continue;
                        }
                        let client = client.clone();
                        let slots = slots.clone();
                        requests.spawn(async move {
                            let _slot = slots.acquire_owned().await.map_err(|error| error.to_string())?;
                            authorize_request(&client, &event, scope, main_document).await
                        });
                    }
                    "Target.attachedToTarget" if event.params["targetInfo"]["type"] == "iframe" => {
                        let Some(sid) = event.params["sessionId"].as_str() else { continue; };
                        let Some(frame) = event.params["targetInfo"]["targetId"].as_str() else { continue; };
                        if let Err(error) = prepare_page(client, sid, observer.retains_diagnostics()).await {
                            if is_session_detached_error(&error) { continue; }
                            return Err(error);
                        }
                        state.frame_sessions.write().await.insert(frame.to_owned(), sid.to_owned());
                        client.send_command_no_wait("Runtime.runIfWaitingForDebugger", None, Some(sid)).await?;
                    }
                    "Target.detachedFromTarget" => {
                        let detached = event.params["sessionId"].as_str();
                        if detached == Some(session) { return Ok(()); }
                        state.frame_sessions.write().await.retain(|_, sid| Some(sid.as_str()) != detached);
                        state.frame_contexts.write().await.retain(|_, frame| Some(frame.session.as_str()) != detached);
                    }
                    "Runtime.executionContextCreated" => {
                        let context = &event.params["context"];
                        if context.pointer("/auxData/isDefault") == Some(&Value::Bool(true))
                            && let (Some(frame), Some(session), Some(unique), Some(id)) = (
                                context.pointer("/auxData/frameId").and_then(Value::as_str),
                                event.session_id.as_deref(), context["uniqueId"].as_str(), context["id"].as_i64(),
                            )
                        {
                            state.frame_contexts.write().await.insert(frame.to_owned(), FrameContext {
                                session: session.to_owned(), unique_id: unique.to_owned(), id,
                            });
                        }
                    }
                    "Runtime.executionContextDestroyed" => {
                        let id = event.params["executionContextId"].as_i64();
                        let unique = event.params["executionContextUniqueId"].as_str();
                        state.frame_contexts.write().await.retain(|_, frame| {
                            Some(frame.session.as_str()) != event.session_id.as_deref()
                                || unique.map_or(Some(frame.id) != id, |unique| frame.unique_id != unique)
                        });
                    }
                    "Runtime.executionContextsCleared" => {
                        state.frame_contexts.write().await.retain(|_, frame| {
                            Some(frame.session.as_str()) != event.session_id.as_deref()
                        });
                    }
                    "Page.windowOpen" => {
                        if matches!(observer, PageEvents::Activity { allow_popups: true, .. })
                            && event.params["userGesture"] == true
                            && let Some(scope) = state.active_scope()
                        {
                            scope.popups().announced += 1;
                            scope.popup_changed.notify_one();
                        }
                    }
                    "Page.javascriptDialogOpening" if main => {
                        let dialog = agena_plugin_sdk::activity::browser::BrowserDialog {
                            kind: event.params["type"].as_str().unwrap_or_default().to_owned(),
                            message: utf8_prefix(event.params["message"].as_str().unwrap_or_default(), 1024).to_owned(),
                        };
                        *state.dialog.lock().unwrap_or_else(|error| error.into_inner()) = Some(dialog);
                    }
                    "Page.javascriptDialogClosed" if main => {
                        state.dialog.lock().unwrap_or_else(|error| error.into_inner()).take();
                    }
                    _ => {
                        if main && ((event.method == "Page.frameNavigated" && event.params.pointer("/frame/parentId").is_none())
                            || event.method == "Page.navigatedWithinDocument"
                        ) {
                            state.document_revision.fetch_add(1, Ordering::Release);
                        }
                        if !state.private.load(Ordering::Acquire)
                            && (main || matches!(observer, PageEvents::Activity { .. })
                                && !matches!(event.method.as_str(), "Page.frameNavigated" | "Page.navigatedWithinDocument"))
                        {
                            observer.forward(event)?;
                        }
                    }
                }
            }
        }
    }
}

async fn authorize_request(
    client: &CdpClient,
    event: &CdpEvent,
    scope: Option<Arc<RequestScope>>,
    main_document: bool,
) -> Result<(), String> {
    let Some(scope) = scope else {
        return decide_request(client, event, false).await;
    };
    let url = event
        .params
        .pointer("/request/url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let document = event.params["resourceType"] == "Document";
    let check = async {
        match &scope.authority {
            RequestAuthority::Agent { context, policy } => {
                host_api::run_in_isolated_host_callback_context(
                    context.clone(),
                    permission_wait::timeout(COMMAND_TIMEOUT, policy(url, document)),
                )
                .await
            }
            RequestAuthority::Human(policy) => {
                permission_wait::timeout(COMMAND_TIMEOUT, policy(url, document)).await
            }
        }
    };
    let result = tokio::select! {
        biased;
        _ = scope.finished.cancelled() => return decide_request(client, event, false).await,
        result = check => result.unwrap_or_else(|_| Err(PluginError::from_kind(PluginErrorKind::Timeout, "Browser request authorization timed out"))),
    };
    let allowed = result.is_ok() && !scope.finished.is_cancelled();
    if let Err(error) = result {
        deny_document(Some(&scope), main_document, error);
    }
    decide_request(client, event, allowed).await
}

fn deny_document(scope: Option<&RequestScope>, main_document: bool, error: PluginError) {
    if let Some(scope) = scope.filter(|_| main_document) {
        scope
            .denied_document
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_or_insert(error);
    }
}

async fn decide_request(client: &CdpClient, event: &CdpEvent, allowed: bool) -> Result<(), String> {
    let id = event.params["requestId"]
        .as_str()
        .ok_or("Browser request has no identity")?;
    let (method, params) = if allowed {
        ("Fetch.continueRequest", json!({"requestId": id}))
    } else {
        (
            "Fetch.failRequest",
            json!({"requestId": id, "errorReason": "BlockedByClient"}),
        )
    };
    match client
        .send_command(method, Some(params), event.session_id.as_deref())
        .await
    {
        Ok(_) => Ok(()),
        // Chrome can cancel a request or detach its frame during a host approval.
        Err(error)
            if is_session_detached_error(&error) || error.contains("Invalid InterceptionId") =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    async fn next_decision(decisions: &mut mpsc::Receiver<String>) -> Option<String> {
        tokio::time::timeout(Duration::from_secs(5), decisions.recv())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn browser_requests_use_only_the_active_tool_authority() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://{}/devtools/browser/test",
            listener.local_addr().unwrap()
        );
        let (emit, mut pending_events) = mpsc::channel::<Value>(1);
        let (observe, mut decisions) = mpsc::channel(1);
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(socket).await.unwrap();
            loop {
                tokio::select! {
                    event = pending_events.recv() => {
                        let Some(event) = event else { break };
                        socket.send(Message::Text(event.to_string().into())).await.unwrap();
                    }
                    incoming = socket.next() => {
                        let Some(Ok(Message::Text(message))) = incoming else { break };
                        let command: Value = serde_json::from_str(&message).unwrap();
                        let method = command["method"].as_str().unwrap();
                        let result = if method == "Target.attachToTarget" {
                            let attached = json!({
                                "method": "Target.attachedToTarget",
                                "params": {"sessionId": "page", "targetInfo": {"type": "page", "targetId": "target"}},
                            });
                            socket.send(Message::Text(attached.to_string().into())).await.unwrap();
                            json!({"sessionId": "page"})
                        } else {
                            json!({})
                        };
                        let reply = json!({"id": command["id"], "result": result});
                        socket.send(Message::Text(reply.to_string().into())).await.unwrap();
                        if matches!(method, "Fetch.continueRequest" | "Fetch.failRequest") {
                            observe.send(method.to_owned()).await.unwrap();
                        }
                    }
                }
            }
        });
        let calls = Arc::new(SyncMutex::new(Vec::new()));
        let policy: BrowserRequestPolicy = Arc::new({
            let calls = calls.clone();
            move |url, _| {
                let calls = calls.clone();
                Box::pin(async move {
                    let current = host_api::current_host_callback_context().unwrap();
                    calls.lock().unwrap().push(current.call_id.unwrap());
                    if url.ends_with("/deny") {
                        return Err(PluginError::from_kind(
                            PluginErrorKind::UserDeclined,
                            "test decline",
                        ));
                    }
                    Ok(())
                })
            }
        });
        let (events, _receiver) = mpsc::channel(1);
        let page = BrowserDriver::connect_page(
            &endpoint,
            "target",
            PageEvents::Activity {
                sender: events,
                capture_console: false,
                allow_popups: false,
            },
        )
        .await
        .unwrap();
        let pause = |suffix: &str, frame: &str| {
            json!({
                "method": "Fetch.requestPaused", "sessionId": "page",
                "params": {"requestId": suffix, "frameId": frame, "resourceType": "Document", "request": {"url": format!("https://example.test/{suffix}")}},
            })
        };
        emit.send(pause("between-calls", "target")).await.unwrap();
        assert_eq!(
            next_decision(&mut decisions).await.as_deref(),
            Some("Fetch.failRequest")
        );
        assert!(calls.lock().unwrap().is_empty());
        for (id, suffix, frame, expected) in [
            (41, "deny", "target", "Fetch.failRequest"),
            (42, "deny", "child", "Fetch.failRequest"),
            (43, "allow", "target", "Fetch.continueRequest"),
        ] {
            host_api::run_in_isolated_host_callback_context(
                host_api::HostCallbackContext {
                    session_id: Some(31),
                    call_id: Some(id),
                    ..Default::default()
                },
                async {
                    let _authorization = page.authorize(policy.clone()).unwrap();
                    emit.send(pause(suffix, frame)).await.unwrap();
                    assert_eq!(
                        next_decision(&mut decisions).await.as_deref(),
                        Some(expected)
                    );
                    let reply = page.command("Page.getNavigationHistory", json!({})).await;
                    if suffix == "deny" && frame == "target" {
                        assert_eq!(reply.unwrap_err().kind, PluginErrorKind::UserDeclined);
                    } else {
                        reply.unwrap();
                    }
                },
            )
            .await;
        }
        assert_eq!(*calls.lock().unwrap(), [41, 42, 43]);
        drop(page);
        server.abort();
    }
}
