use super::*;
use serde_json::{Value, json};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use uuid::Uuid;

const MAX_CONTEXTS: usize = 64;

#[derive(PartialEq, Eq)]
pub(super) enum ContextOwner {
    Session(BrowserOwner),
    Workspace(PathBuf),
    Bot(Uuid),
    Global,
}

impl ContextOwner {
    fn scope(&self) -> BrowserContextScope {
        match self {
            Self::Session(_) => BrowserContextScope::Session,
            Self::Workspace(_) => BrowserContextScope::Workspace,
            Self::Bot(_) => BrowserContextScope::Bot,
            Self::Global => BrowserContextScope::Global,
        }
    }

    pub(super) fn visible(&self, caller: &BrowserOwner, bot: Option<Uuid>) -> bool {
        match self {
            Self::Session(owner) => owner == caller,
            Self::Workspace(workspace) => workspace == &caller.workspace,
            Self::Bot(owner) => Some(*owner) == bot,
            Self::Global => true,
        }
    }
}

pub(in crate::browser::plugin) struct BrowserContext {
    pub(crate) id: String,
    pub(crate) endpoint: String,
    pub(super) root: BrowserDriver,
    pub(super) owner: ContextOwner,
    name: Option<String>,
    // A human takeover holds the write guard for its duration. Page calls
    // hold a read guard so retiring a context cannot race an active action.
    gate: Arc<RwLock<bool>>,
    keep_alive: Option<ChromeLease>,
}

impl BrowserContext {
    pub(super) fn access(&self) -> SdkResult<OwnedRwLockReadGuard<bool>> {
        let guard = self.gate.clone().try_read_owned().map_err(|_| busy())?;
        if *guard || self.root.is_closed() {
            return Err(missing_context());
        }
        Ok(guard)
    }

    fn try_exclusive(&self) -> SdkResult<OwnedRwLockWriteGuard<bool>> {
        self.gate.clone().try_write_owned().map_err(|_| busy())
    }

    pub(super) async fn take_control(
        &self,
        timeout: Duration,
    ) -> SdkResult<OwnedRwLockWriteGuard<bool>> {
        let guard = tokio::time::timeout(timeout, self.gate.clone().write_owned())
            .await
            .map_err(|_| busy())?;
        if *guard || self.root.is_closed() {
            return Err(missing_context());
        }
        Ok(guard)
    }

    pub(super) async fn observe_close(&self) -> Option<OwnedRwLockReadGuard<bool>> {
        let guard = self.gate.clone().read_owned().await;
        (!*guard).then_some(guard)
    }

    fn summary(&self) -> Value {
        json!({
            "context_id": self.id,
            "scope": self.owner.scope(),
            "name": self.name,
            "default": self.name.is_none(),
            "keep_alive": self.keep_alive.is_some(),
        })
    }
}

fn busy() -> PluginError {
    PluginError::from_kind(
        PluginErrorKind::PolicyDenied,
        "The browser context is busy or under human control. Retry when it is available.",
    )
}

fn missing_context() -> PluginError {
    PluginError::invalid_params(
        "Browser context is unavailable. Use browser.contexts to find an accessible context.",
    )
}

/// Retire a CDP context if the caller was cancelled while Chrome created it.
struct CreatingContext {
    driver: BrowserDriver,
    id: Option<String>,
}

impl Drop for CreatingContext {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let driver = self.driver.clone();
            tokio::spawn(async move {
                if let Err(error) = driver.dispose_context(&id).await
                    && !driver.is_closed()
                {
                    log_secondary_plugin_failure("retire incomplete browser context", &error);
                }
            });
        }
    }
}

impl BrowserRoot {
    async fn get_or_create_context(
        &mut self,
        owner: ContextOwner,
        name: Option<String>,
        keep_alive: Option<ChromeLease>,
    ) -> SdkResult<(Arc<BrowserContext>, bool)> {
        if let Some(context) = self
            .contexts
            .values()
            .find(|context| context.owner == owner && context.name == name)
        {
            return Ok((context.clone(), false));
        }
        if self.contexts.len() >= MAX_CONTEXTS {
            return Err(PluginError::invalid_params(
                "Browser context quota reached. Use browser.close_context to free a context.",
            ));
        }
        let driver = self.driver.clone();
        let mut created = tokio::spawn(async move {
            let id = driver.create_context().await?;
            Ok::<_, PluginError>(CreatingContext {
                driver,
                id: Some(id),
            })
        })
        .await
        .map_err(|error| PluginError::internal_error(&error))??;
        let context = Arc::new(BrowserContext {
            id: created.id.take().expect("Chrome returned a context ID"),
            endpoint: self.endpoint.clone(),
            root: self.driver.clone(),
            owner,
            name,
            gate: Arc::default(),
            keep_alive,
        });
        self.contexts.insert(context.id.clone(), context.clone());
        Ok((context, true))
    }
}

impl BrowserSessions {
    async fn live_contexts(&self) -> Vec<Arc<BrowserContext>> {
        self.root
            .lock()
            .await
            .as_ref()
            .filter(|root| !root.driver.is_closed())
            .map(|root| root.contexts.values().cloned().collect())
            .unwrap_or_default()
    }

    async fn live_context(&self, id: &str) -> SdkResult<Arc<BrowserContext>> {
        self.root
            .lock()
            .await
            .as_ref()
            .filter(|root| !root.driver.is_closed())
            .and_then(|root| root.contexts.get(id))
            .cloned()
            .ok_or_else(missing_context)
    }

    pub(super) async fn private_contexts(&self, owner: &BrowserOwner) -> Vec<Arc<BrowserContext>> {
        self.live_contexts().await.into_iter()
            .filter(|context| matches!(&context.owner, ContextOwner::Session(session) if session == owner))
            .collect()
    }

    pub(super) async fn close_context(
        self: &Arc<Self>,
        context: Arc<BrowserContext>,
        host: Arc<dyn HostClient>,
        message: &'static str,
    ) -> SdkResult<Vec<String>> {
        let mut guard = context.try_exclusive()?;
        let state = self.clone();
        tokio::spawn(async move {
            state
                .dispose_context(&context, &mut guard, host.as_ref(), message)
                .await
        })
        .await
        .map_err(|error| PluginError::internal_error(&error))?
    }

    pub(super) async fn dispose_context(
        &self,
        context: &BrowserContext,
        closed: &mut bool,
        host: &dyn HostClient,
        message: &str,
    ) -> SdkResult<Vec<String>> {
        if *closed {
            return Ok(Vec::new());
        }
        {
            let mut root = self.root.lock().await;
            if let Some(root) = root
                .as_mut()
                .filter(|root| root.contexts.contains_key(&context.id))
            {
                if !root.driver.is_closed()
                    && let Err(error) = root.driver.dispose_context(&context.id).await
                    && !root.driver.is_closed()
                    && !error
                        .diagnostic_message()
                        .contains("Failed to find context")
                {
                    return Err(error);
                }
                root.contexts.remove(&context.id);
            }
            *closed = true;
        }
        let pages = self.remove_context_pages(&context.id).await;
        for page in &pages {
            page.driver.disconnect();
            self.content.lock().await.remove_page(&page.id);
            page.publish(BackgroundActivityStatus::Stopped, Some(message), host)
                .await;
        }
        Ok(pages.iter().map(|page| page.id.clone()).collect())
    }

    async fn remove_context_pages(&self, id: &str) -> Vec<Arc<BrowserPage>> {
        let mut pages = self.pages.lock().await;
        let ids: Vec<_> = pages
            .values()
            .filter(|page| page.context.id == id)
            .map(|page| page.id.clone())
            .collect();
        ids.iter().filter_map(|id| pages.remove(id)).collect()
    }
}

impl BrowserPlugin {
    async fn bot_id(&self) -> SdkResult<Option<Uuid>> {
        let identity = self.state()?.host.bot_context().await?;
        match identity["enabled"].as_bool() {
            Some(false) => Ok(None),
            Some(true) => identity["bot_id"]
                .as_str()
                .and_then(|id| id.parse().ok())
                .map(Some)
                .ok_or_else(|| PluginError::internal("Invalid Bot identity from the host")),
            None => Err(PluginError::internal("Invalid Bot context from the host")),
        }
    }

    pub(super) async fn can_access_context(
        &self,
        caller: &BrowserOwner,
        context: &BrowserContext,
    ) -> SdkResult<bool> {
        let bot = if matches!(&context.owner, ContextOwner::Bot(_)) {
            self.bot_id().await?
        } else {
            None
        };
        Ok(context.owner.visible(caller, bot))
    }

    pub(super) async fn accessible_context(
        &self,
        caller: &BrowserOwner,
        id: &str,
    ) -> SdkResult<Arc<BrowserContext>> {
        let context = self.browser_state.live_context(id).await?;
        if !self.can_access_context(caller, &context).await? {
            return Err(missing_context());
        }
        Ok(context)
    }

    pub(super) async fn visible_contexts(
        &self,
        caller: &BrowserOwner,
        filter: Option<&str>,
    ) -> SdkResult<Vec<Arc<BrowserContext>>> {
        if let Some(id) = filter {
            return Ok(vec![self.accessible_context(caller, id).await?]);
        }
        let mut contexts = self.browser_state.live_contexts().await;
        let bot = if contexts
            .iter()
            .any(|context| matches!(&context.owner, ContextOwner::Bot(_)))
        {
            self.bot_id().await?
        } else {
            None
        };
        contexts.retain(|context| context.owner.visible(caller, bot));
        Ok(contexts)
    }

    pub(super) async fn ensure_root<'a>(
        &self,
        root: &'a mut Option<BrowserRoot>,
    ) -> SdkResult<&'a mut BrowserRoot> {
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
            let host = self.browser_state.activity_host(&self.state()?.host);
            popups::watch(&self.browser_state, &driver, host).await?;
            *root = Some(BrowserRoot {
                endpoint,
                driver,
                contexts: BTreeMap::new(),
            });
        }
        Ok(root.as_mut().expect("Chrome root initialized"))
    }

    pub(super) async fn default_context(
        &self,
        caller: BrowserOwner,
    ) -> SdkResult<Arc<BrowserContext>> {
        let mut root = self.browser_state.root.lock().await;
        let (context, _) = self
            .ensure_root(&mut root)
            .await?
            .get_or_create_context(ContextOwner::Session(caller), None, None)
            .await?;
        Ok(context)
    }

    pub(in crate::browser::plugin) async fn list_contexts(
        &self,
        call: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        let caller = browser_owner(call)?;
        let contexts: Vec<_> = self
            .visible_contexts(&caller, None)
            .await?
            .iter()
            .map(|context| context.summary())
            .collect();
        let summary = format!("{} browser contexts", contexts.len());
        let data = json!({"contexts": contexts});
        Ok(ToolInvokeOutput::from_parts(
            "List browser contexts",
            summary,
            data.to_string(),
            Some(data),
            Default::default(),
            Vec::new(),
        ))
    }

    pub(in crate::browser::plugin) async fn create_browser_context(
        &self,
        call: &ToolInvokeContext<'_>,
        input: &BrowserContextCreateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let _control = handoff::agent_access(self.browser_state.control(call.session_id)).await?;
        let caller = browser_owner(call)?;
        let owner = match input.scope {
            BrowserContextScope::Session => ContextOwner::Session(caller),
            BrowserContextScope::Workspace if !caller.workspace.as_os_str().is_empty() => {
                ContextOwner::Workspace(caller.workspace)
            }
            BrowserContextScope::Workspace => {
                return Err(PluginError::invalid_params(
                    "This conversation has no workspace",
                ));
            }
            BrowserContextScope::Bot => {
                ContextOwner::Bot(self.bot_id().await?.ok_or_else(|| {
                    PluginError::invalid_params("This conversation belongs to no AI Bot")
                })?)
            }
            BrowserContextScope::Global => ContextOwner::Global,
        };
        let _lease = self.browser_state.lease()?;
        let keep_alive = input
            .keep_alive
            .then(|| self.browser_state.lease())
            .transpose()?;
        let mut root = self.browser_state.root.lock().await;
        let (context, created) = self
            .ensure_root(&mut root)
            .await?
            .get_or_create_context(owner, Some(input.name.clone()), keep_alive)
            .await?;
        let mut data = context.summary();
        data["created"] = json!(created);
        let summary = if created {
            "Browser context created"
        } else {
            "Browser context reused"
        };
        Ok(ToolInvokeOutput::from_parts(
            "Create browser context",
            summary,
            data.to_string(),
            Some(data),
            Default::default(),
            Vec::new(),
        ))
    }

    pub(in crate::browser::plugin) async fn close_browser_context(
        &self,
        call: &ToolInvokeContext<'_>,
        id: &str,
    ) -> SdkResult<ToolInvokeOutput> {
        let _control = handoff::agent_access(self.browser_state.control(call.session_id)).await?;
        let context = self.accessible_context(&browser_owner(call)?, id).await?;
        let _lease = self.browser_state.lease()?;
        let host = self.browser_state.activity_host(&self.state()?.host);
        let closed = self
            .browser_state
            .close_context(context, host, "Browser context closed.")
            .await?;
        Ok(browser_output(
            "Close browser context",
            "Browser context closed".into(),
            json!({"context_id": id, "closed_sessions": closed}),
        ))
    }
}
