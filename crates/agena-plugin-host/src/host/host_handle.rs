impl HostHandle {
    pub fn new(inner: Arc<dyn HostClient>) -> Self {
        Self::new_with_registry(
            inner,
            Arc::new(RwLock::new(PluginToolRegistry::new())),
            Arc::new(RwLock::new(HashMap::new())),
        )
    }

    pub fn new_with_registry(
        inner: Arc<dyn HostClient>,
        tool_registry: Arc<RwLock<PluginToolRegistry>>,
        plugin_indices: Arc<RwLock<HashMap<PluginKey, usize>>>,
    ) -> Self {
        Self::new_with_components(
            inner,
            tool_registry,
            plugin_indices,
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(crate::status::StatusRegistry::new()),
            Arc::new(PluginLogStore::default()),
            None,
        )
    }

    pub fn new_with_components(
        inner: Arc<dyn HostClient>,
        tool_registry: Arc<RwLock<PluginToolRegistry>>,
        plugin_indices: Arc<RwLock<HashMap<PluginKey, usize>>>,
        plugin_names: Arc<RwLock<HashMap<PluginKey, String>>>,
        statuses: Arc<crate::status::StatusRegistry>,
        logs: Arc<PluginLogStore>,
        callback_base_url: Option<String>,
    ) -> Self {
        Self {
            inner: tokio::sync::RwLock::new(inner),


            tokens: tokio::sync::Mutex::new(HashMap::new()),
            callback_base_url,
            tool_registry,
            plugin_indices,
            plugin_names,
            hook_catalog: Arc::new(RwLock::new(BTreeMap::new())),
            tool_registry_events: Arc::new(RwLock::new(VecDeque::new())),
            tool_registry_event_listener: Arc::new(RwLock::new(None)),
            statuses,
            logs,
            statusline: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            themes: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            quotas: Arc::new(crate::quota::QuotaRegistry::default()),
            plugin_transports: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    pub fn quota_registry(&self) -> Arc<crate::quota::QuotaRegistry> {
        Arc::clone(&self.quotas)
    }

    pub fn install_quota_registry(&mut self, registry: Arc<crate::quota::QuotaRegistry>) {
        self.quotas = registry;
    }

    /// Register a plugin transport so the handle can route streaming events.
    pub async fn register_plugin_transport(
        &self,
        plugin_id: PluginKey,
        transport: Arc<dyn PluginTransport>,
    ) {
        self.plugin_transports
            .write()
            .await
            .insert(plugin_id, transport);
    }

    pub async fn ingest_stream_event_for_plugin(
        &self,
        plugin_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<bool, PluginError> {
        let Some(plugin_key) = plugin_id.parse().ok() else {
            return Ok(false);
        };
        let transport = self
            .plugin_transports
            .read()
            .await
            .get(&plugin_key)
            .cloned();
        let Some(transport) = transport else {
            return Ok(false);
        };
        transport
            .ingest_stream_event(method, params)
            .await
            .map_err(transport_to_plugin_error)
    }

    pub fn status_registry(&self) -> Arc<crate::status::StatusRegistry> {
        Arc::clone(&self.statuses)
    }

    pub fn log_store(&self) -> Arc<PluginLogStore> {
        Arc::clone(&self.logs)
    }

    pub fn set_plugin_hook_catalog(&self, registration: HostHookRegistration) {
        if let Ok(mut catalog) = self.hook_catalog.write() {
            catalog.insert(registration.plugin_id.clone(), registration);
        }
    }

    pub fn set_tool_registry_event_listener(&self, listener: Option<ToolRegistryEventListener>) {
        if let Ok(mut slot) = self.tool_registry_event_listener.write() {
            *slot = listener;
        }
    }

    pub fn latest_tool_registry_event(&self) -> Option<ToolRegistryChangedEvent> {
        self.tool_registry_events
            .read()
            .ok()
            .and_then(|events| events.back().cloned())
    }

    pub fn tool_registry_events_since(
        &self,
        after_generation: Option<u64>,
        limit: usize,
    ) -> Vec<ToolRegistryChangedEvent> {
        let limit = limit.clamp(1, 500);
        self.tool_registry_events
            .read()
            .map(|events| {
                events
                    .iter()
                    .filter(|event| {
                        after_generation
                            .map(|generation| event.generation > generation)
                            .unwrap_or(true)
                    })
                    .rev()
                    .take(limit)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn record_tool_registry_event(&self, event: ToolRegistryChangedEvent) {
        let listener = self
            .tool_registry_event_listener
            .read()
            .ok()
            .and_then(|slot| slot.clone());
        if let Ok(mut events) = self.tool_registry_events.write() {
            events.push_back(event.clone());
            while events.len() > 256 {
                events.pop_front();
            }
        }
        if let Some(listener) = listener {
            listener(event);
        }
    }

    pub fn append_plugin_log(
        &self,
        plugin_id: impl Into<String>,
        level: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
        fields: serde_json::Value,
    ) -> Option<PluginLogRecord> {
        let plugin_id = plugin_id.into();
        let plugin_key = plugin_id.parse().ok()?;
        Some(
            self.logs
                .append(&plugin_key, level, source, message, fields),
        )
    }

    pub fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<PluginLogRecord> {
        let Some(plugin_key) = plugin_id.parse().ok() else {
            return Vec::new();
        };
        self.logs.list(&plugin_key, after_seq, limit)
    }

    /// Replace the underlying [`HostClient`] live (used after the runtime is
    /// constructed and we can install the real implementation).
    pub async fn install_client(&self, client: Arc<dyn HostClient>) {
        *self.inner.write().await = client;
    }

    pub fn set_plugin_manifest_name(&self, plugin_id: PluginKey, plugin_name: impl Into<String>) {
        if let Ok(mut names) = self.plugin_names.write() {
            names.insert(plugin_id, plugin_name.into());
        }
    }

    /// Remove every in-memory contribution made while a plugin was
    /// initializing. Plugins may call host APIs from `meta/init`, so a failed
    /// load must be rolled back before the host continues with other plugins.
    pub async fn rollback_failed_plugin(&self, plugin_id: &PluginKey) {
        self.tokens.lock().await.remove(plugin_id);
        self.plugin_transports.write().await.remove(plugin_id);
        if let Ok(mut indices) = self.plugin_indices.write() {
            indices.remove(plugin_id);
        }
        if let Ok(mut names) = self.plugin_names.write() {
            names.remove(plugin_id);
        }
        if let Ok(mut hooks) = self.hook_catalog.write() {
            hooks.remove(plugin_id);
        }
        if let Ok(mut tools) = self.tool_registry.write() {
            tools.remove_plugin(plugin_id);
        }
        if let Ok(mut events) = self.tool_registry_events.write() {
            events.retain(|event| &event.plugin != plugin_id);
        }
        if let Ok(mut statusline) = self.statusline.write() {
            statusline.retain(|(owner, _), _| owner != plugin_id);
        }
        if let Ok(mut themes) = self.themes.write() {
            themes.retain(|_, theme| &theme.plugin_id != plugin_id);
        }
        self.quotas.remove_plugin(plugin_id);
    }

    pub fn callback_url(&self, plugin_id: &str) -> Option<String> {
        self.callback_base_url
            .as_ref()
            .map(|base| format!("{}/plugin-rpc/{}", base.trim_end_matches('/'), plugin_id))
    }

    pub async fn callback_token(&self, plugin_id: &str) -> Option<String> {
        self.callback_base_url.as_ref()?;
        let plugin_key: PluginKey = plugin_id.parse().ok()?;
        let mut tokens = self.tokens.lock().await;
        Some(
            tokens
                .entry(plugin_key)
                .or_insert_with(|| format!("cb-{}", uuid::Uuid::new_v4().simple()))
                .clone(),
        )
    }

    pub async fn validate_callback_token(&self, plugin_id: &str, token: Option<&str>) -> bool {
        let Some(token) = token else {
            return false;
        };
        let Some(plugin_key) = plugin_id.parse().ok() else {
            return false;
        };
        let tokens = self.tokens.lock().await;
        tokens
            .get(&plugin_key)
            .is_some_and(|expected| expected == token)
    }

    pub fn scoped_host_client(
        self: &Arc<Self>,
        plugin_id: impl Into<String>,
    ) -> Arc<dyn HostClient> {
        Arc::new(ScopedHostClient {
            handle: Arc::clone(self),
            plugin_id: plugin_id.into(),
        })
    }

    pub fn host_handler_for(
        self: &Arc<Self>,
        plugin_id: impl Into<String>,
    ) -> crate::transport::stdio::HostHandler {
        let this = Arc::clone(self);
        let plugin_id = plugin_id.into();
        Arc::new(move |method: String, params: serde_json::Value| {
            let this = Arc::clone(&this);
            let plugin_id = plugin_id.clone();
            Box::pin(async move {
                this.handle_call_for_plugin(plugin_id.as_str(), &method, params)
                    .await
            })
        })
    }

    pub async fn handle_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        self.handle_call_for_plugin("", method, params).await
    }

    pub async fn handle_call_for_plugin(
        &self,
        plugin_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        let inner = self.inner.read().await.clone();
        let plugin_id = (!plugin_id.is_empty()).then(|| plugin_id.to_string());
        let callback_context = callback_context_from_params(&params);
        // Per-plugin quota guard. Skipped for callbacks that aren't tied to
        // any plugin (i.e. handle_call without a plugin_id) since those
        // can't be attributed to a quota bucket.
        let _quota_guard = match plugin_id.as_deref() {
            Some(pid) => {
                let plugin_key: PluginKey = pid.parse().map_err(|err| {
                    PluginError::internal(format_args!("invalid plugin id `{pid}`: {err}"))
                        .with_hook(method)
                        .with_plugin(pid)
                })?;
                Some(self.quotas.acquire(&plugin_key).map_err(|err| {
                    PluginError::internal(err)
                        .with_hook(method)
                        .with_plugin(pid)
                })?)
            }
            None => None,
        };
        host_api::run_in_host_callback_context(
            scoped_context(plugin_id.clone(), callback_context),
            async {
                match method {
                    method::HOST_LOG => {
                        let p: HostLogParams = parse(params)?;
                        host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, None),
                            inner.log(p.level, p.message, p.fields),
                        )
                        .await;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_EVENT_PUBLISH => {
                        
                        let env: EventEnvelope = parse(params)?;
                        host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, None),
                            inner.publish_event(env),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_EVENT_SUBSCRIBE => {
                        
                        let p: HostSubscribeParams = parse(params)?;
                        let sub: EventSubscription = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, None),
                            inner.subscribe_events(p.filter),
                        )
                        .await?;
                        Ok(serde_json::json!({ "subscription_id": sub.id }))
                    }
                    method::HOST_EVENT_UNSUBSCRIBE => {
                        
                        let p: HostUnsubscribeParams = parse(params)?;
                        host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, None),
                            inner.unsubscribe_events(p.subscription_id),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_CONFIG_READ => {
                        
                        let p: HostConfigReadParams = parse(params)?;
                        host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.read_config(p.path),
                        )
                        .await
                    }
                    method::HOST_CONFIG_RELOAD => {
                        
                        let p: HostConfigReloadParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.reload_config(),
                        )
                        .await?;
                        serde_json::to_value(out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_TOOL_INVOKE => {
                        
                        let p: HostInvokeToolParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.invoke_tool(p.tool, p.input),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_ASK_USER => {
                        
                        let p: HostAskUserParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.ask_user(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SUBTASK_RUN => {
                        
                        let p: HostRunSubtaskParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.run_subtask(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SUBTASK_CANCEL => {
                        
                        let p: HostCancelSubtaskParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.cancel_subtask(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SUBTASK_MESSAGE => {
                        
                        let p: HostMessageSubtaskParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.message_subtask(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SUBTASK_OUTPUT => {
                        
                        let p: HostReadSubtaskOutputParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.read_subtask_output(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_TOOL_LIST => {
                        
                        let p: HostListToolsParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.list_tools(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_CONTEXT_STATUS => {
                        
                        let p: HostContextStatusParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.get_context_status(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SESSION_SET_MODEL => {
                        
                        let p: HostSetSessionModelParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.set_session_model(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_IMAGE_EXECUTE => {
                        
                        let p: HostImageExecuteParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.image_execute(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SNAPSHOT_ENTER => {
                        
                        let p: HostEnterSnapshotParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.enter_snapshot(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SNAPSHOT_EXIT => {
                        
                        let p: HostExitSnapshotParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.exit_snapshot(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MONITOR_START => {
                        
                        let p: HostMonitorStartParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.monitor_start(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MONITOR_LIST => {
                        
                        let p: HostMonitorListParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.monitor_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MONITOR_READ => {
                        
                        let p: HostMonitorReadParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.monitor_read(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MONITOR_STOP => {
                        
                        let p: HostMonitorStopParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.monitor_stop(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_TOOL_REGISTRY_REGISTER => {
                        
                        let p: HostToolRegisterParams = parse(params)?;
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("tool.registry.register requires plugin id")
                        })?;
                        let response = self.tool_upsert_for_plugin(&plugin_id, p.request.tool)?;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_TOOL_REGISTRY_UPDATE => {
                        
                        let p: HostToolUpdateParams = parse(params)?;
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("tool.registry.update requires plugin id")
                        })?;
                        let response = self.tool_upsert_for_plugin(&plugin_id, p.request.tool)?;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_TOOL_REGISTRY_REMOVE => {
                        
                        let p: HostToolRemoveParams = parse(params)?;
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("tool.registry.remove requires plugin id")
                        })?;
                        let response = self.tool_remove_for_plugin(
                            &plugin_id,
                            &p.request.name,
                            p.request.by_model_name,
                        )?;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_TOOL_REGISTRY_LIST => {
                        
                        let response = self.registered_tool_list_response()?;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_STORAGE_GET => {
                        
                        let p: HostStorageGetParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.storage_get(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_STORAGE_SET => {
                        
                        let p: HostStorageSetParams = parse(params)?;
                        host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.storage_set(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_STORAGE_DELETE => {
                        
                        let p: HostStorageDeleteParams = parse(params)?;
                        host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.storage_delete(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_STORAGE_LIST => {
                        
                        let p: HostStorageListParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.storage_list(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SECRET_GET => {
                        
                        let p: HostSecretGetParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.secret_get(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SECRET_SET => {
                        
                        let p: HostSecretSetParams = parse(params)?;
                        host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.secret_set(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_SECRET_DELETE => {
                        
                        let p: HostSecretDeleteParams = parse(params)?;
                        host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.secret_delete(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_SECRET_LIST => {
                        
                        let p: HostSecretListParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.secret_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PLUGIN_STATUS_LIST => {
                        
                        let response = self.plugin_status_list_response();
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_PLUGIN_STATUS_GET => {
                        
                        let p: HostPluginStatusGetParams = parse(params)?;
                        let response = self.plugin_status_get_response(&p.request.plugin_id);
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_LSP_LIST_SERVERS => {
                        
                        let p: HostLspListServersParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.lsp_list_servers(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_LSP_LIST_DIAGNOSTICS => {
                        
                        let p: HostLspListDiagnosticsParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.lsp_list_diagnostics(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SNAPSHOT_LIST => {
                        
                        let p: HostSnapshotListParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.snapshot_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SCHEDULER_LIST => {
                        
                        let p: HostSchedulerListParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.scheduler_list(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SCHEDULER_CREATE => {
                        
                        let p: HostSchedulerCreateParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.scheduler_create(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_SCHEDULER_DELETE => {
                        
                        let p: HostSchedulerDeleteParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.scheduler_delete(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_HOOK_LIST => {
                        
                        let response = self.hook_list_response().await;
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MCP_LIST_SERVERS => {
                        
                        let p: HostMcpListServersParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.mcp_list_servers(),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_MCP_ADD_SERVER => {
                        
                        let p: HostMcpAddServerParams = parse(params)?;
                        host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.mcp_add_server(p.request),
                        )
                        .await?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_MCP_REMOVE_SERVER => {
                        
                        let p: HostMcpRemoveServerParams = parse(params)?;
                        let out = host_api::run_in_host_callback_context(
                            scoped_context(plugin_id, p.context),
                            inner.mcp_remove_server(p.request),
                        )
                        .await?;
                        serde_json::to_value(&out)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_UI_STATUSLINE_CONTRIBUTE => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.statusline.contribute requires plugin id")
                        })?;
                                                let p: HostStatuslineContributeParams = parse(params)?;
                        self.statusline_contribute(&plugin_id, p.request);
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_UI_STATUSLINE_LIST => {
                        
                        let response = self.statusline_list_response();
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_UI_STATUSLINE_REMOVE => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.statusline.remove requires plugin id")
                        })?;
                                                let p: HostStatuslineRemoveParams = parse(params)?;
                        let removed = self.statusline_remove(&plugin_id, &p.request.segment_id);
                        serde_json::to_value(&HostStatuslineRemoveResponse { removed })
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_UI_THEME_REGISTER => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.theme.register requires plugin id")
                        })?;
                                                let p: HostThemeRegisterParams = parse(params)?;
                        self.theme_register(&plugin_id, p.request)?;
                        Ok(serde_json::Value::Object(Default::default()))
                    }
                    method::HOST_UI_THEME_LIST => {
                        
                        let response = self.theme_list_response();
                        serde_json::to_value(&response)
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    method::HOST_UI_THEME_REMOVE => {
                        let plugin_id = plugin_id.ok_or_else(|| {
                            host_unavailable("ui.theme.remove requires plugin id")
                        })?;
                                                let p: HostThemeRemoveParams = parse(params)?;
                        let removed = self.theme_remove(&plugin_id, &p.request.id);
                        serde_json::to_value(&HostThemeRemoveResponse { removed })
                            .map_err(|e| PluginError::invalid_params(e.to_string()))
                    }
                    other => Err(PluginError::not_implemented(other)),
                }
            },
        )
        .await
    }

    pub(super) fn tool_upsert_for_plugin(
        &self,
        plugin_id: &str,
        definition: crate::sdk::ToolDefinition,
    ) -> Result<HostToolMutationResponse, PluginError> {
        let plugin_key: PluginKey = plugin_id.parse()?;
        let registered = self
            .plugin_indices
            .read()
            .map_err(|_| host_unavailable("plugin index lock poisoned"))?
            .contains_key(&plugin_key);
        if !registered {
            return Err(host_unavailable(format!(
                "plugin `{plugin_id}` is not registered"
            )));
        }
        validate_tool_definition(&plugin_key, &definition).map_err(PluginError::invalid_params)?;
        let mut tool_registry = self
            .tool_registry
            .write()
            .map_err(|_| host_unavailable("tool registry lock poisoned"))?;
        let plugin_tool_name = definition.name.clone();
        let kind = if tool_registry
            .lookup_for_plugin(&plugin_key, &plugin_tool_name)
            .is_some()
        {
            ToolRegistryChangeKind::Updated
        } else {
            ToolRegistryChangeKind::Registered
        };
        let tool = tool_registry
            .upsert_from_plugin(&plugin_key, definition)
            .map_err(PluginError::invalid_params)?;
        let event = ToolRegistryChangedEvent {
            kind,
            generation: tool_registry.generation(),
            timestamp_ms: unix_timestamp_ms(),
            plugin: tool.plugin_key().clone(),
            tool_key: tool.tool_key().clone(),
            tool: Some(tool.definition.clone()),
        };
        self.record_tool_registry_event(event.clone());
        Ok(HostToolMutationResponse {
            generation: tool_registry.generation(),
            model_name: Some(tool.canonical_name()),
            tool: Some(tool.definition.clone()),
            event: Some(event),
        })
    }

    pub(super) fn tool_remove_for_plugin(
        &self,
        plugin_id: &str,
        name: &str,
        by_model_name: bool,
    ) -> Result<HostToolMutationResponse, PluginError> {
        let mut tool_registry = self
            .tool_registry
            .write()
            .map_err(|_| host_unavailable("tool registry lock poisoned"))?;
        let plugin_key: PluginKey = plugin_id.parse()?;
        let tool_name = if by_model_name {
            let tool_key: ToolKey = name.parse()?;
            if tool_key.plugin() != &plugin_key {
                return Err(host_unavailable(format!(
                    "tool `{name}` does not belong to plugin `{plugin_id}`"
                )));
            }
            tool_key.name().to_string()
        } else {
            name.to_string()
        };
        let removed = tool_registry.remove_from_plugin(&plugin_key, tool_name.as_str());
        let event = removed.as_ref().map(|tool| ToolRegistryChangedEvent {
            kind: ToolRegistryChangeKind::Removed,
            generation: tool_registry.generation(),
            timestamp_ms: unix_timestamp_ms(),
            plugin: tool.plugin_key().clone(),
            tool_key: tool.tool_key().clone(),
            tool: Some(tool.definition.clone()),
        });
        if let Some(event) = event.as_ref() {
            self.record_tool_registry_event(event.clone());
        }
        Ok(HostToolMutationResponse {
            generation: tool_registry.generation(),
            model_name: removed.as_ref().map(RegisteredTool::canonical_name),
            tool: removed.map(|tool| tool.definition.clone()),
            event,
        })
    }

    pub(super) fn registered_tool_list_response(
        &self,
    ) -> Result<HostRegisteredToolListResponse, PluginError> {
        let snapshot = self
            .tool_registry
            .read()
            .map_err(|_| host_unavailable("tool registry lock poisoned"))?
            .snapshot();
        let tools = snapshot
            .tools
            .into_iter()
            .map(|tool| HostRegisteredToolDescriptor {
                plugin: tool.plugin_key().clone(),
                tool_key: tool.tool_key().clone(),
                tool: tool.definition.clone(),
            })
            .collect();
        Ok(HostRegisteredToolListResponse {
            generation: snapshot.generation,
            tools,
            last_event: self.latest_tool_registry_event(),
        })
    }

    pub(super) fn plugin_status_list_response(&self) -> HostPluginStatusListResponse {
        HostPluginStatusListResponse {
            statuses: self
                .statuses
                .list()
                .into_iter()
                .map(host_status_from)
                .collect(),
        }
    }

    /// Build the plugin-discovery inventory from the shared plugin registry.
    /// The host_handle does not own the loaded-plugin set directly, so it
    /// reads the plugin names map plus tool registry: plugin_id, version,
    /// summary, tags, and per-plugin tool count are projected here. Plugins
    /// load their own descriptor through the scoped client, which the
    /// host_handle fills from the connection registry.
    pub(super) fn plugin_list_response(&self) -> crate::sdk::HostPluginListResponse {
        let mut plugins = BTreeMap::<String, crate::sdk::HostPluginDescriptor>::new();
        if let Ok(names) = self.plugin_names.read() {
            for (key, _name) in names.iter() {
                plugins.entry(key.to_string()).or_insert_with(|| {
                    crate::sdk::HostPluginDescriptor {
                        plugin_id: key.clone(),
                        summary: None,
                        version: String::new(),
                        tags: Vec::new(),
                        tools: Vec::new(),
                    }
                });
            }
        }
        if let Ok(registry) = self.tool_registry.read() {
            for tool in registry.registered_tools_owned() {
                if let Some(descriptor) = plugins.get_mut(&tool.plugin_key().to_string()) {
                    if !(tool.namespace() == "agena" && tool.plugin_name() == "tools") {
                        descriptor.tools.push(tool.canonical_name());
                    }
                    for tag in &tool.definition.tags {
                        let label = tag.to_string();
                        if !descriptor.tags.contains(&label) {
                            descriptor.tags.push(label);
                        }
                    }
                }
            }
        }
        for descriptor in plugins.values_mut() {
            descriptor.tools.sort();
            descriptor.tools.dedup();
        }
        crate::sdk::HostPluginListResponse {
            plugins: plugins.into_values().collect(),
        }
    }

    pub(super) fn plugin_status_get_response(
        &self,
        plugin_id: &PluginKey,
    ) -> HostPluginStatusGetResponse {
        HostPluginStatusGetResponse {
            status: self.statuses.get(plugin_id).map(host_status_from),
        }
    }

    pub(super) async fn hook_list_response(&self) -> HostHookListResponse {
        let hooks = self
            .hook_catalog
            .read()
            .map(|catalog| catalog.values().cloned().collect())
            .unwrap_or_default();
        HostHookListResponse { hooks }
    }

    pub(super) fn statusline_contribute(
        &self,
        plugin_id: &str,
        req: HostStatuslineContributeRequest,
    ) {
        let Ok(plugin_id) = plugin_id.parse::<PluginKey>() else {
            return;
        };
        if let Ok(mut guard) = self.statusline.write() {
            let key = (plugin_id.clone(), req.segment_id.clone());
            guard.insert(
                key,
                HostStatuslineSegment {
                    plugin_id,
                    segment_id: req.segment_id,
                    content: req.content,
                    priority: req.priority,
                    color: req.color,
                },
            );
        }
    }

    pub(super) fn statusline_remove(&self, plugin_id: &str, segment_id: &str) -> bool {
        let Ok(plugin_id) = plugin_id.parse::<PluginKey>() else {
            return false;
        };
        if let Ok(mut guard) = self.statusline.write() {
            return guard.remove(&(plugin_id, segment_id.to_string())).is_some();
        }
        false
    }

    pub fn statusline_list_response(&self) -> HostStatuslineListResponse {
        let mut segments: Vec<HostStatuslineSegment> = self
            .statusline
            .read()
            .map(|guard| guard.values().cloned().collect())
            .unwrap_or_default();
        segments.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
                .then_with(|| a.segment_id.cmp(&b.segment_id))
        });
        HostStatuslineListResponse { segments }
    }

    pub(super) fn theme_register(
        &self,
        plugin_id: &str,
        req: HostThemeRegisterRequest,
    ) -> Result<(), PluginError> {
        let plugin_id = plugin_id.parse::<PluginKey>()?;
        if req.id.trim().is_empty() || req.id.trim() != req.id {
            return Err(PluginError::invalid_params(
                "theme id must be non-empty and must not contain leading or trailing whitespace",
            ));
        }
        let mut guard = self
            .themes
            .write()
            .map_err(|_| host_unavailable("theme registry lock poisoned"))?;
        let key = (plugin_id.clone(), req.id.clone());
        guard.insert(
            key,
            HostThemePalette {
                id: req.id,
                plugin_id,
                display_name: req.display_name,
                colors: req.colors,
            },
        );
        Ok(())
    }

    pub(super) fn theme_remove(&self, plugin_id: &str, id: &str) -> bool {
        let Ok(plugin_id) = plugin_id.parse::<PluginKey>() else {
            return false;
        };
        if let Ok(mut guard) = self.themes.write() {
            return guard.remove(&(plugin_id, id.to_owned())).is_some();
        }
        false
    }

    pub fn theme_list_response(&self) -> HostThemeListResponse {
        let themes: Vec<HostThemePalette> = self
            .themes
            .read()
            .map(|guard| guard.values().cloned().collect())
            .unwrap_or_default();
        HostThemeListResponse { themes }
    }
}
use super::{
    Arc, BTreeMap, EventEnvelope, EventSubscription, HashMap, HostAskUserParams,
    HostCancelSubtaskParams, HostClient, HostConfigReadParams,
    HostConfigReloadParams, HostContextStatusParams, HostEnterSnapshotParams,
    HostExitSnapshotParams, HostHandle, HostHookListResponse, HostHookRegistration,
    HostImageExecuteParams, HostInvokeToolParams, HostListToolsParams, HostLogParams,
    HostLspListDiagnosticsParams, HostLspListServersParams, HostMcpAddServerParams,
    HostMcpListServersParams, HostMcpRemoveServerParams, HostMessageSubtaskParams,
    HostMonitorListParams, HostMonitorReadParams, HostMonitorStartParams, HostMonitorStopParams,
    HostPluginStatusGetParams, HostPluginStatusGetResponse, HostPluginStatusListResponse,
    HostReadSubtaskOutputParams, HostRegisteredToolDescriptor, HostRegisteredToolListResponse,
    HostRunSubtaskParams, HostSchedulerCreateParams, HostSchedulerDeleteParams,
    HostSchedulerListParams, HostSecretDeleteParams, HostSecretGetParams, HostSecretListParams,
    HostSecretSetParams, HostSetSessionModelParams, HostSnapshotListParams,
    HostStatuslineContributeParams, HostStatuslineContributeRequest, HostStatuslineListResponse,
    HostStatuslineRemoveParams, HostStatuslineRemoveResponse, HostStatuslineSegment,
    HostStorageDeleteParams, HostStorageGetParams, HostStorageListParams, HostStorageSetParams,
    HostSubscribeParams, HostThemeListResponse, HostThemePalette, HostThemeRegisterParams,
    HostThemeRegisterRequest, HostThemeRemoveParams, HostThemeRemoveResponse,
    HostToolMutationResponse, HostToolRegisterParams, HostToolRemoveParams, HostToolUpdateParams,
    HostUnsubscribeParams, PluginError, PluginKey, PluginLogRecord,
    PluginLogStore, PluginToolRegistry, PluginTransport, RegisteredTool, RwLock, ScopedHostClient,
    ToolKey, ToolRegistryChangeKind, ToolRegistryChangedEvent, ToolRegistryEventListener, VecDeque,
    callback_context_from_params, host_api, host_status_from, host_unavailable, method, parse,
    scoped_context, transport_to_plugin_error, unix_timestamp_ms, validate_tool_definition,
};
