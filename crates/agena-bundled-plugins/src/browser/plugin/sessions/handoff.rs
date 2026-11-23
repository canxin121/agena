//! The operator and the agent take turns owning a conversation's browser.

mod viewport;

use std::sync::Arc;

use agena_plugin_host::sdk::{host_api::AskUserRequest, permission_wait};
use agena_plugin_sdk::browser::{
    BrowserControlCommand, BrowserControlResult, BrowserHelpRequest, BrowserNetworkAuthority,
    BrowserTab,
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, DropGuard};
use uuid::Uuid;

use super::*;

const CONTROL_TIME: Duration = Duration::from_secs(600);
pub(crate) const TAKEOVER_INPUT_KIND: &str = "browser_takeover";
const RESUME_ANSWER: &str = "Resume browser task";

pub(super) type Control = RwLock<ControlState>;
pub(super) type AgentAccess = OwnedRwLockReadGuard<ControlState>;

#[derive(Default)]
pub(super) struct ControlState {
    pub(super) controller: tokio::sync::OnceCell<Arc<dyn BrowserController>>,
    human: Option<HumanControl>,
    help: Option<HelpRequest>,
}

struct HumanControl {
    id: Uuid,
    expires: Instant,
    context: Arc<BrowserContext>,
    _authorizations: Vec<BrowserAuthorization>,
    exclusive: OwnedRwLockWriteGuard<bool>,
    _chrome: ChromeLease,
    _deadline: DropGuard,
}

struct HelpRequest {
    id: Uuid,
    page_id: String,
    reason: String,
    returned_page: Option<String>,
}

impl ControlState {
    fn shows_page(&self, session_id: i64, page: &BrowserPage) -> bool {
        if let Some(human) = &self.human {
            return Arc::ptr_eq(&human.context, &page.context);
        }
        page.owner.session_id == Some(session_id)
            || self.help.as_ref().is_some_and(|help| {
                help.page_id == page.id || help.returned_page.as_ref() == Some(&page.id)
            })
    }

    pub(super) fn waiting_status(&self) -> Option<&'static str> {
        if self.human.is_some() {
            Some(
                "The user is controlling this browser in Agena Web. Wait for them to return control.",
            )
        } else if let Some(help) = &self.help {
            Some(if help.returned_page.is_some() {
                "The user returned the browser. The task is waiting for their resume command."
            } else {
                "The browser is waiting for the user to take control in Agena Web."
            })
        } else {
            None
        }
    }

    fn check_id(&self, id: &str, check_expiry: bool) -> SdkResult<&HumanControl> {
        let id = Uuid::parse_str(id).ok();
        self.human
            .as_ref()
            .filter(|control| {
                Some(control.id) == id && (!check_expiry || Instant::now() < control.expires)
            })
            .ok_or_else(control_denied)
    }
}

pub(super) async fn agent_access(control: Arc<Control>) -> SdkResult<AgentAccess> {
    let guard = control.read_owned().await;
    if let Some(message) = guard.waiting_status() {
        return Err(PluginError::from_kind(
            PluginErrorKind::PolicyDenied,
            message,
        ));
    }
    Ok(guard)
}

pub(super) fn closed() -> PluginError {
    PluginError::from_kind(
        PluginErrorKind::Disconnected,
        "This browser page is no longer open",
    )
}

fn control_denied() -> PluginError {
    PluginError::from_kind(
        PluginErrorKind::PolicyDenied,
        "Browser control belongs to a different view or has expired",
    )
}

impl BrowserSessions {
    pub(super) fn control(&self, session_id: i64) -> Arc<Control> {
        let mut controls = self
            .controls
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(control) = controls.get(&session_id).and_then(std::sync::Weak::upgrade) {
            return control;
        }
        controls.retain(|_, control| control.strong_count() != 0);
        let control = Arc::new(Control::default());
        controls.insert(session_id, Arc::downgrade(&control));
        control
    }

    async fn operator_pages(
        &self,
        session_id: i64,
        control: &ControlState,
    ) -> Vec<Arc<BrowserPage>> {
        self.pages
            .lock()
            .await
            .values()
            .filter(|page| control.shows_page(session_id, page) && !page.driver.is_closed())
            .cloned()
            .collect()
    }

    async fn operator_page(
        &self,
        session_id: i64,
        page_id: &str,
        control: &ControlState,
    ) -> SdkResult<Arc<BrowserPage>> {
        self.pages
            .lock()
            .await
            .get(page_id)
            .filter(|page| control.shows_page(session_id, page) && !page.driver.is_closed())
            .cloned()
            .ok_or_else(closed)
    }

    pub(super) async fn operator_command(
        self: &Arc<Self>,
        session_id: i64,
        command: BrowserControlCommand,
        network: Option<Arc<dyn BrowserNetworkAuthority>>,
        host: Arc<dyn HostClient>,
        fake_ip_ranges: &[String],
    ) -> SdkResult<BrowserControlResult> {
        let control = self.control(session_id);
        match command {
            BrowserControlCommand::List => {
                let control = control.read().await;
                let mut tabs = Vec::new();
                for page in self.operator_pages(session_id, &control).await {
                    let title = if control.human.is_some() {
                        format!("Tab {}", tabs.len() + 1)
                    } else {
                        let Ok(_context) = page.context.access() else {
                            continue;
                        };
                        page.activity.lock().await.title.clone()
                    };
                    tabs.push(BrowserTab {
                        page_id: page.id.clone(),
                        opener_page_id: page.opener_id.clone(),
                        title,
                    });
                }
                Ok(BrowserControlResult::State {
                    tabs,
                    human_control: control.human.is_some(),
                    help: control.help.as_ref().map(|help| BrowserHelpRequest {
                        page_id: help.page_id.clone(),
                        reason: help.reason.clone(),
                        ready_to_resume: control.human.is_none() && help.returned_page.is_some(),
                    }),
                })
            }
            BrowserControlCommand::Frame {
                page_id,
                control_id,
                thumbnail,
            } => {
                let control = control.read().await;
                if control.human.is_some() || control_id.is_some() {
                    control.check_id(control_id.as_deref().unwrap_or_default(), true)?;
                }
                let _lease = self.lease()?;
                let page = self.operator_page(session_id, &page_id, &control).await?;
                let _context = control
                    .human
                    .is_none()
                    .then(|| page.context.access())
                    .transpose()?;
                page.verify_preview_lease().await?;
                viewport::frame(&page, thumbnail).await
            }
            BrowserControlCommand::TakeControl { page_id } => {
                let network = network.ok_or_else(control_denied)?;
                let (id, ended) = self
                    .take_control(session_id, &page_id, &control, network, fake_ip_ranges)
                    .await?;
                let state = Arc::downgrade(self);
                tokio::spawn(async move {
                    tokio::select! {
                        _ = ended.cancelled() => return,
                        _ = tokio::time::sleep(CONTROL_TIME) => {},
                    }
                    if let Some(state) = state.upgrade() {
                        let mut guard = control.write().await;
                        if guard.human.as_ref().is_some_and(|human| human.id == id)
                            && let Err(error) = state.discard(&mut guard, host.as_ref()).await
                        {
                            log_secondary_plugin_failure("expire browser control", &error);
                        }
                    }
                });
                Ok(BrowserControlResult::Control {
                    control_id: id.to_string(),
                })
            }
            BrowserControlCommand::Input {
                page_id,
                control_id,
                events,
            } => {
                let control = control.read().await;
                control.check_id(&control_id, true)?;
                let page = self.operator_page(session_id, &page_id, &control).await?;
                let _gate = self.lock_page(&page).await?;
                page.verify_preview_lease().await?;
                viewport::input(&page, events).await?;
                Ok(BrowserControlResult::Done)
            }
            BrowserControlCommand::ReturnControl {
                control_id,
                page_id,
            } => {
                let mut control = control.write().await;
                control.check_id(&control_id, true)?;
                let pages = self.operator_pages(session_id, &control).await;
                let returned_page = page_id
                    .as_ref()
                    .or_else(|| control.help.as_ref().map(|help| &help.page_id))
                    .and_then(|id| pages.iter().find(|page| &page.id == id))
                    .or_else(|| pages.first())
                    .map(|page| page.id.clone())
                    .ok_or_else(closed)?;
                tokio::time::timeout(Duration::from_secs(8), async {
                    for page in &pages {
                        viewport::check_for_credentials(page).await?;
                        page.driver.make_private()?;
                    }
                    Ok::<_, PluginError>(())
                })
                .await
                .map_err(|_| {
                    PluginError::from_kind(
                        PluginErrorKind::Timeout,
                        "Chrome did not finish checking the pages. Try again or discard the pages.",
                    )
                })??;
                control.human.take();
                if let Some(help) = &mut control.help {
                    help.returned_page = Some(returned_page);
                }
                Ok(BrowserControlResult::Done)
            }
            BrowserControlCommand::Close { control_id } => {
                let mut control = control.write_owned().await;
                control.check_id(&control_id, false)?;
                let state = self.clone();
                tokio::spawn(async move { state.discard(&mut control, host.as_ref()).await })
                    .await
                    .map_err(|error| PluginError::internal_error(&error))??;
                Ok(BrowserControlResult::Done)
            }
        }
    }

    async fn take_control(
        &self,
        session_id: i64,
        page_id: &str,
        control: &Control,
        network: Arc<dyn BrowserNetworkAuthority>,
        fake_ip_ranges: &[String],
    ) -> SdkResult<(Uuid, CancellationToken)> {
        let mut control = tokio::time::timeout(Duration::from_secs(8), control.write())
            .await
            .map_err(|_| {
                PluginError::from_kind(
                    PluginErrorKind::Timeout,
                    "A browser action is still running. Try taking control again when it finishes.",
                )
            })?;
        if control.human.is_some() {
            return Err(control_denied());
        }
        let chrome = self.lease()?;
        let target = self.operator_page(session_id, page_id, &control).await?;
        let context = target.context.clone();
        let exclusive = context.take_control(Duration::from_secs(8)).await?;
        self.operator_page(session_id, page_id, &control)
            .await?
            .verify_preview_lease()
            .await?;
        let pages = self
            .pages
            .lock()
            .await
            .values()
            .filter(|page| Arc::ptr_eq(&page.context, &context) && !page.driver.is_closed())
            .cloned()
            .collect::<Vec<_>>();
        let mut authorizations = Vec::new();
        for page in pages {
            // Suppression starts before a single operator input can reach CDP.
            page.driver.make_private()?;
            self.content.lock().await.remove_page(&page.id);
            let policy = operator_policy(page.preview.clone(), fake_ip_ranges, network.clone());
            authorizations.push(page.driver.authorize_human(policy)?);
        }
        let id = Uuid::new_v4();
        let ended = CancellationToken::new();
        control.human = Some(HumanControl {
            id,
            expires: Instant::now() + CONTROL_TIME,
            context,
            _authorizations: authorizations,
            exclusive,
            _chrome: chrome,
            _deadline: ended.clone().drop_guard(),
        });
        if let Some(help) = &mut control.help {
            help.returned_page = None;
        }
        Ok((id, ended))
    }

    async fn discard(&self, control: &mut ControlState, host: &dyn HostClient) -> SdkResult<()> {
        if let Some(human) = &mut control.human {
            self.dispose_context(
                &human.context,
                &mut human.exclusive,
                host,
                "Browser control ended.",
            )
            .await?;
        }
        control.human.take();
        Ok(())
    }
}

fn operator_policy(
    preview: Option<ManagedPreviewTarget>,
    fake_ip_ranges: &[String],
    network: Arc<dyn BrowserNetworkAuthority>,
) -> BrowserRequestPolicy {
    let fake_ip_ranges = Arc::<[String]>::from(fake_ip_ranges);
    Arc::new(move |raw, document| {
        let network = network.clone();
        let preview = preview.clone();
        let fake_ip_ranges = fake_ip_ranges.clone();
        Box::pin(async move {
            let url = prepare_fetch_url(&raw).map_err(PluginError::invalid_params)?;
            if managed_preview::within_scope(preview.as_ref(), &url, document).await? {
                let preview = preview.expect("scope matching requires a preview");
                network
                    .require(preview.upstream_url().as_str(), &[])
                    .await?;
                return preview.validate().await.map_err(managed_preview::denied);
            }
            network.require(url.as_str(), &[]).await?;
            let addresses =
                network_policy::resolve_browser_addresses(&url, &fake_ip_ranges).await?;
            network.require(url.as_str(), &addresses).await
        })
    })
}

struct PendingHelp {
    control: Arc<Control>,
    id: Uuid,
}

impl Drop for PendingHelp {
    fn drop(&mut self) {
        let control = self.control.clone();
        let id = self.id;
        tokio::spawn(async move {
            let mut guard = control.write().await;
            if guard.help.as_ref().is_some_and(|help| help.id == id) {
                guard.help.take();
            }
        });
    }
}

impl BrowserPlugin {
    pub(in crate::browser::plugin) async fn request_takeover(
        &self,
        context: &ToolInvokeContext<'_>,
        page_id: &str,
        reason: &str,
    ) -> SdkResult<ToolInvokeOutput> {
        if context.session_id <= 0 {
            return Err(PluginError::invalid_params(
                "A conversation session is required",
            ));
        }
        let control = self.browser_state.control(context.session_id);
        let id = Uuid::new_v4();
        let _lease = self.browser_state.lease()?;
        {
            let mut guard = control.write().await;
            if guard.human.is_some() || guard.help.is_some() {
                return Err(PluginError::invalid_params(
                    "Browser takeover is already active or pending",
                ));
            }
            let caller = browser_owner(context)?;
            let page = self.visible_page(&caller, page_id).await?;
            let _context = page.context.access()?;
            let _page = self.browser_state.lock_page(&page).await?;
            self.require_page_access(&page).await?;
            self.ensure_browser_controller(&guard).await?;
            guard.help = Some(HelpRequest {
                id,
                page_id: page.id.clone(),
                reason: reason.to_owned(),
                returned_page: None,
            });
        }
        let pending = PendingHelp {
            control: control.clone(),
            id,
        };
        let response = permission_wait::waiting(self.state()?.host.ask_user(AskUserRequest {
            title: "Browser takeover".into(),
            kind: TAKEOVER_INPUT_KIND.into(),
            prompt: format!(
                "{} Open Browser in Agena Web to help, then return control.",
                reason
            ),
            options: vec![RESUME_ANSWER.into()],
            auto_resolution_ms: Some(CONTROL_TIME.as_millis() as u64),
            ..Default::default()
        }))
        .await?;
        let mut guard = control.write().await;
        let returned = guard
            .help
            .take()
            .filter(|help| {
                help.id == id
                    && guard.human.is_none()
                    && !response.cancelled
                    && !response.timed_out
                    && response.reply == RESUME_ANSWER
            })
            .and_then(|help| help.returned_page);
        drop(guard);
        drop(pending);
        let (status, message) = if returned.is_some() {
            (
                "returned",
                "The user returned control. Take a fresh snapshot to check the page.",
            )
        } else {
            (
                "cancelled",
                "The browser takeover request ended without a completed handback. Check browser.list before taking any browser action.",
            )
        };
        Ok(browser_output(
            "Browser takeover",
            message.into(),
            serde_json::json!({"status": status, "session_id": returned.as_deref().unwrap_or(page_id)}),
        ))
    }
}
