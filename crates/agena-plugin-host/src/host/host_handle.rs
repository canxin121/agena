pub(super) struct CallbackAuthorityLease {
    authorities: Arc<Mutex<HashMap<String, CallbackAuthorityRecord>>>,
    token: String,
}

impl Drop for CallbackAuthorityLease {
    fn drop(&mut self) {
        if let Ok(mut authorities) = self.authorities.lock() {
            authorities.remove(&self.token);
        }
    }
}

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
            scoped_tools: Arc::new(ScopedRegistry::new()),
            operation_registry: Arc::new(ScopedRegistry::new()),
            plugin_indices,
            plugin_names,
            hook_catalog: Arc::new(RwLock::new(BTreeMap::new())),
            tool_registry_events: Arc::new(RwLock::new(VecDeque::new())),
            tool_registry_event_listener: Arc::new(RwLock::new(None)),
            statuses,
            logs,
            display: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            host_notifications: Arc::new(RwLock::new(std::collections::VecDeque::new())),
            themes: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            quotas: Arc::new(crate::quota::QuotaRegistry::default()),
            plugin_transports: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            service_bindings: tokio::sync::RwLock::new(BTreeMap::new()),
            effect_scopes: RwLock::new(HashMap::new()),
            callback_authorities: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn own_manifest_resources(
        &self,
        plugin_id: &PluginKey,
        manifest: &crate::sdk::PluginManifest,
    ) -> Result<(), PluginError> {
        if !manifest.tools.is_empty() {
            let registry = Arc::clone(&self.tool_registry);
            let owned_plugin = plugin_id.clone();
            self.replace_effect_sync(
                plugin_id,
                "host.manifest.tools",
                "manifest".to_string(),
                move || {
                    registry
                        .write()
                        .map_err(|_| "tool registry lock poisoned".to_string())?
                        .remove_plugin(&owned_plugin);
                    Ok(())
                },
            )?;
        }
        let effect_scope = self.ensure_effect_scope(plugin_id);
        for operation in &manifest.operations {
            let registry_name = operation_registry_name(plugin_id, operation.id.as_str());
            let item = PluginOperationCatalogItem {
                plugin_id: plugin_id.clone(),
                accepts_empty_input: operation.input.default_value().is_ok(),
                default_input: operation
                    .input
                    .default_value()
                    .unwrap_or(serde_json::Value::Null),
                operation: operation.clone(),
            };
            self.operation_registry
                .register_with_effect_kind(
                    &effect_scope,
                    None,
                    registry_name,
                    item,
                    "host.operation",
                    operation.id.clone(),
                )
                .map_err(|error| host_unavailable(error.to_string()))?;
        }
        for export in &manifest.services.exports {
            self.replace_effect_sync(
                plugin_id,
                "service.export",
                format!("{}@v{}", export.id, export.api_version),
                || Ok(()),
            )?;
        }
        for import in &manifest.services.imports {
            self.replace_effect_sync(
                plugin_id,
                "service.import",
                format!("{}@v{}", import.id, import.api_version),
                || Ok(()),
            )?;
        }
        for contribution in &manifest.surface.display {
            self.replace_effect_sync(
                plugin_id,
                "host.manifest.display",
                contribution.id.clone(),
                || Ok(()),
            )?;
        }
        for theme in &manifest.surface.terminal.themes {
            self.replace_effect_sync(
                plugin_id,
                "host.manifest.theme",
                theme.id.clone(),
                || Ok(()),
            )?;
        }
        Ok(())
    }

    pub fn quota_registry(&self) -> Arc<crate::quota::QuotaRegistry> {
        Arc::clone(&self.quotas)
    }

    pub fn operation_registry(&self) -> Arc<ScopedRegistry<String, PluginOperationCatalogItem>> {
        Arc::clone(&self.operation_registry)
    }

    pub fn scoped_tool_registry(&self) -> Arc<ScopedRegistry<ToolKey, RegisteredTool>> {
        Arc::clone(&self.scoped_tools)
    }

    pub(super) fn dispose_tool_scope(&self, scope: &PluginScopeKey) {
        for removed in self.scoped_tools.clear_scope_tree(scope) {
            let tool = removed.value;
            let generation = self
                .tool_registry
                .write()
                .map(|mut registry| registry.touch_generation())
                .unwrap_or_default();
            self.record_tool_registry_event(ToolRegistryChangedEvent {
                kind: ToolRegistryChangeKind::Removed,
                generation,
                timestamp_ms: unix_timestamp_ms(),
                plugin: tool.plugin_key().clone(),
                tool_key: tool.tool_key().clone(),
                scope: match removed.layer {
                    crate::scoped_registry::ScopedRegistryLayer::Scope { scope } => {
                        Some(scope.to_string())
                    }
                    crate::scoped_registry::ScopedRegistryLayer::Global => None,
                },
                tool: Some(tool.definition.clone()),
            });
        }
    }

    pub fn install_quota_registry(&mut self, registry: Arc<crate::quota::QuotaRegistry>) {
        self.quotas = registry;
    }

    pub fn begin_plugin_instance(&self, plugin_id: PluginKey) -> Arc<PluginEffectScope> {
        let scope = PluginEffectScope::new(plugin_id.clone());
        self.effect_scopes
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(plugin_id, Arc::clone(&scope));
        scope
    }

    fn ensure_effect_scope(&self, plugin_id: &PluginKey) -> Arc<PluginEffectScope> {
        if let Some(scope) = self
            .effect_scopes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(plugin_id)
            .filter(|scope| scope.state() == crate::effect_scope::PluginEffectScopeState::Active)
            .cloned()
        {
            return scope;
        }
        self.begin_plugin_instance(plugin_id.clone())
    }

    pub fn is_current_effect_scope(
        &self,
        plugin_id: &PluginKey,
        scope: &Arc<PluginEffectScope>,
    ) -> bool {
        self.effect_scopes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(plugin_id)
            .is_some_and(|current| Arc::ptr_eq(current, scope))
    }

    pub fn effect_scope(&self, plugin_id: &PluginKey) -> Option<Arc<PluginEffectScope>> {
        self.effect_scopes
            .read()
            .expect("effect scope registry lock")
            .get(plugin_id)
            .cloned()
    }

    pub fn effect_scope_inspect(&self, plugin_id: &PluginKey) -> Option<PluginEffectScopeInspect> {
        self.effect_scopes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(plugin_id)
            .map(|scope| scope.inspect())
    }

    fn replace_effect_sync<F>(
        &self,
        plugin_id: &PluginKey,
        kind: &'static str,
        label: String,
        disposer: F,
    ) -> Result<(), PluginError>
    where
        F: FnOnce() -> Result<(), String> + Send + 'static,
    {
        let scope = self.ensure_effect_scope(plugin_id);
        scope.release(kind, label.as_str());
        scope
            .own_sync(kind, label, disposer)
            .map(|_| ())
            .map_err(|error| host_unavailable(error.to_string()))
    }

    fn replace_effect_async<F, Fut>(
        &self,
        plugin_id: &PluginKey,
        kind: &'static str,
        label: String,
        disposer: F,
    ) -> Result<(), PluginError>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let scope = self.ensure_effect_scope(plugin_id);
        scope.release(kind, label.as_str());
        scope
            .own_async(kind, label, disposer)
            .map(|_| ())
            .map_err(|error| host_unavailable(error.to_string()))
    }

    fn release_effect(&self, plugin_id: &PluginKey, kind: &str, label: &str) -> bool {
        self.effect_scopes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(plugin_id)
            .is_some_and(|scope| scope.release(kind, label))
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
            .insert(plugin_id.clone(), transport);
        let transports = Arc::clone(&self.plugin_transports);
        let owned_plugin = plugin_id.clone();
        if let Err(error) = self.replace_effect_async(
            &plugin_id,
            "host.transport",
            "primary".to_string(),
            move || async move {
                transports.write().await.remove(&owned_plugin);
                Ok(())
            },
        ) {
            self.logs.append(
                &plugin_id,
                "error",
                "effects",
                format!("failed to own plugin transport: {error}"),
                serde_json::Value::Null,
            );
        }
    }

    pub async fn install_service_bindings(
        &self,
        bindings: BTreeMap<PluginServiceBindingKey, PluginServiceBinding>,
    ) {
        *self.service_bindings.write().await = bindings;
    }

    pub async fn service_bindings(
        &self,
    ) -> BTreeMap<PluginServiceBindingKey, PluginServiceBinding> {
        self.service_bindings.read().await.clone()
    }

    /// Run one Host→Plugin call with a short-lived opaque callback authority.
    /// Plugin callbacks may echo this token only while the originating future
    /// is alive; the trusted context is reconstructed host-side and the token
    /// is revoked synchronously when this call exits.
    pub(crate) async fn run_in_authorized_callback_context<F, T>(
        &self,
        plugin_id: &PluginKey,
        context: HostCallbackContext,
        future: F,
    ) -> T
    where
        F: std::future::Future<Output = T>,
    {
        let (context, _lease) = self.issue_callback_authority(plugin_id, context);
        host_api::run_in_host_callback_context(context, future).await
    }

    pub(super) fn issue_callback_authority(
        &self,
        plugin_id: &PluginKey,
        mut context: HostCallbackContext,
    ) -> (HostCallbackContext, CallbackAuthorityLease) {
        let generation = self
            .effect_scope(plugin_id)
            .map(|scope| scope.generation())
            .unwrap_or_default();
        context.plugin_id = Some(plugin_id.to_string());
        context.authority_token = None;
        let token = format!("ctx-{}", uuid::Uuid::new_v4().simple());
        self.callback_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                token.clone(),
                CallbackAuthorityRecord {
                    plugin_id: plugin_id.clone(),
                    generation,
                    context: context.clone(),
                },
            );
        context.authority_token = Some(token.clone());
        (
            context,
            CallbackAuthorityLease {
                authorities: Arc::clone(&self.callback_authorities),
                token,
            },
        )
    }

    pub(super) fn validated_callback_context(
        &self,
        attributed_plugin_id: Option<String>,
        context: Option<HostCallbackContext>,
    ) -> Result<HostCallbackContext, PluginError> {
        let plugin_key = attributed_plugin_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(str::parse::<PluginKey>)
            .transpose()
            .map_err(|error| PluginError::invalid_params(error.to_string()))?;
        let plugin_only = HostCallbackContext {
            plugin_id: attributed_plugin_id.clone(),
            ..HostCallbackContext::default()
        };
        let Some(context) = context else {
            return Ok(plugin_only);
        };
        let privileged = context.session_id.is_some()
            || context.call_id.is_some()
            || context.workspace_root.is_some()
            || context.tool_name.is_some();
        let Some(token) = context.authority_token.as_deref() else {
            if privileged {
                return Err(PluginError::from_kind(
                    PluginErrorKind::PolicyDenied,
                    "plugin callback supplied privileged context without a host authority token",
                ));
            }
            return Ok(plugin_only);
        };
        let record = self
            .callback_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(token)
            .cloned()
            .ok_or_else(|| {
                PluginError::from_kind(
                    PluginErrorKind::PolicyDenied,
                    "plugin callback authority token is unknown or expired",
                )
            })?;
        let Some(plugin_key) = plugin_key else {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "privileged plugin callback requires transport-attributed plugin identity",
            ));
        };
        if record.plugin_id != plugin_key {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "plugin callback authority token belongs to a different plugin",
            ));
        }
        if context.session_id != record.context.session_id
            || context.call_id != record.context.call_id
            || context.workspace_root != record.context.workspace_root
            || context.tool_name != record.context.tool_name
        {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "plugin callback context does not match its host authority token",
            ));
        }
        let current_generation = self
            .effect_scope(&plugin_key)
            .map(|scope| scope.generation())
            .ok_or_else(|| {
                PluginError::from_kind(
                    PluginErrorKind::PolicyDenied,
                    "plugin callback authority has no active effect generation",
                )
            })?;
        if current_generation != record.generation {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "plugin callback authority belongs to a stale plugin generation",
            ));
        }
        let mut trusted = record.context;
        trusted.plugin_id = Some(plugin_key.to_string());
        trusted.authority_token = Some(token.to_string());
        Ok(trusted)
    }

    pub async fn invoke_service_for_plugin(
        &self,
        consumer: &str,
        request: PluginServiceInvokeInput,
        context: Option<HostCallbackContext>,
    ) -> Result<PluginServiceInvokeOutput, PluginError> {
        let context = self.validated_callback_context(Some(consumer.to_string()), context)?;
        request
            .validate()
            .map_err(|error| PluginError::invalid_params(error.to_string()))?;
        let key = PluginServiceBindingKey {
            consumer: consumer.to_string(),
            service: request.service.clone(),
            api_version: request.api_version,
        };
        let binding = self
            .service_bindings
            .read()
            .await
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                PluginError::invalid_params(format!(
                    "plugin `{consumer}` did not resolve service `{}` API v{}",
                    request.service, request.api_version
                ))
            })?;
        let method_contract = binding
            .methods
            .get(&request.method)
            .cloned()
            .ok_or_else(|| {
                PluginError::invalid_params(format!(
                    "service `{}` API v{} does not declare method `{}`",
                    request.service, request.api_version, request.method
                ))
            })?;
        method_contract
            .input
            .validate_value(&request.input)
            .map_err(|error| {
                PluginError::invalid_params(format!(
                    "service `{}@v{}::{}` input is invalid: {error}",
                    request.service, request.api_version, request.method
                ))
            })?;
        let provider_key: PluginKey = binding.provider.parse().map_err(|error| {
            PluginError::internal(format!(
                "resolved service provider `{}` is invalid: {error}",
                binding.provider
            ))
        })?;
        let transport = self
            .plugin_transports
            .read()
            .await
            .get(&provider_key)
            .cloned()
            .ok_or_else(|| {
                PluginError::from_kind(
                    PluginErrorKind::HostUnavailable,
                    format!(
                        "service provider `{}` is not active for `{}` API v{}",
                        binding.provider, request.service, request.api_version
                    ),
                )
            })?;
        let params = serde_json::to_value(&request)
            .map_err(|error| PluginError::invalid_params(error.to_string()))?;
        let call = self.run_in_authorized_callback_context(
            &provider_key,
            context,
            transport.dispatch(method::SERVICE_INVOKE, params),
        );
        let value = tokio::time::timeout(std::time::Duration::from_secs(30), call)
            .await
            .map_err(|_| {
                PluginError::internal(format!(
                    "service `{}` API v{} invocation timed out",
                    request.service, request.api_version
                ))
            })?
            .map_err(transport_to_plugin_error)?;
        method_contract
            .output
            .validate_value(&value)
            .map_err(|error| {
                PluginError::internal(format!(
                    "service provider `{}` returned invalid output for `{}@v{}::{}`: {error}",
                    binding.provider, request.service, request.api_version, request.method
                ))
            })?;
        Ok(PluginServiceInvokeOutput {
            provider: binding.provider,
            output: value,
        })
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
        let plugin_id = registration.plugin_id.clone();
        if let Ok(mut catalog) = self.hook_catalog.write() {
            catalog.insert(plugin_id.clone(), registration);
        }
        let catalog = Arc::clone(&self.hook_catalog);
        let owned_plugin = plugin_id.clone();
        if let Err(error) = self.replace_effect_sync(
            &plugin_id,
            "host.hooks",
            "manifest".to_string(),
            move || {
                catalog
                    .write()
                    .map_err(|_| "hook catalog lock poisoned".to_string())?
                    .remove(&owned_plugin);
                Ok(())
            },
        ) {
            self.logs.append(
                &plugin_id,
                "error",
                "effects",
                format!("failed to own hook catalog registration: {error}"),
                serde_json::Value::Null,
            );
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

    fn latest_visible_tool_registry_event(
        &self,
        scope: Option<&PluginScopeKey>,
    ) -> Option<ToolRegistryChangedEvent> {
        self.tool_registry_events.read().ok().and_then(|events| {
            events
                .iter()
                .rev()
                .find(|event| tool_registry_event_visible_in_scope(event, scope))
                .cloned()
        })
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
    /// initializing. Exact scope identity prevents an older generation from
    /// deleting registrations created by a newer instance with the same id.
    pub async fn rollback_failed_plugin_for_scope(
        &self,
        plugin_id: &PluginKey,
        scope: &Arc<PluginEffectScope>,
    ) {
        self.dispose_plugin_resources_for_scope(plugin_id, scope)
            .await;
    }

    pub async fn rollback_failed_plugin(&self, plugin_id: &PluginKey) {
        self.dispose_plugin_resources(plugin_id).await;
    }

    /// Idempotently release the exact effect scope associated with one plugin
    /// generation. Plugin-wide fallback cleanup runs only when that scope is
    /// still the current generation after asynchronous disposal completes.
    pub async fn dispose_plugin_resources_for_scope(
        &self,
        plugin_id: &PluginKey,
        scope: &Arc<PluginEffectScope>,
    ) {
        let report = scope.dispose().await;
        for error in report.errors {
            self.logs.append(
                plugin_id,
                "error",
                "effects",
                format!(
                    "plugin effect disposal failed for generation {}: {error}",
                    scope.generation()
                ),
                serde_json::json!({ "generation": scope.generation() }),
            );
        }
        if !self.is_current_effect_scope(plugin_id, scope) {
            return;
        }
        self.tokens.lock().await.remove(plugin_id);
        self.plugin_transports.write().await.remove(plugin_id);
        self.service_bindings.write().await.retain(|key, binding| {
            key.consumer != plugin_id.to_string() && binding.provider != plugin_id.to_string()
        });
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
        if let Ok(mut display) = self.display.write() {
            display.retain(|(owner, _), _| owner != plugin_id);
        }
        if let Ok(mut themes) = self.themes.write() {
            themes.retain(|_, theme| &theme.plugin_id != plugin_id);
        }
        if let Ok(mut notifications) = self.host_notifications.write() {
            notifications.retain(|notification| notification.plugin_id != plugin_id.to_string());
        }
        self.quotas.remove_plugin(plugin_id);
    }

    pub async fn dispose_plugin_resources(&self, plugin_id: &PluginKey) {
        let scope = self.effect_scope(plugin_id);
        if let Some(scope) = scope {
            self.dispose_plugin_resources_for_scope(plugin_id, &scope)
                .await;
        }
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
        let plugin_id = plugin_id.into();
        let plugin_key: PluginKey = plugin_id
            .parse()
            .expect("scoped host client requires a validated plugin id");
        let effect_scope = self.ensure_effect_scope(&plugin_key);
        self.scoped_host_client_for_scope(plugin_id, effect_scope)
    }

    pub fn scoped_host_client_for_scope(
        self: &Arc<Self>,
        plugin_id: impl Into<String>,
        effect_scope: Arc<PluginEffectScope>,
    ) -> Arc<dyn HostClient> {
        let plugin_id = plugin_id.into();
        let plugin_key: PluginKey = plugin_id
            .parse()
            .expect("scoped host client requires a validated plugin id");
        debug_assert_eq!(effect_scope.plugin_id(), &plugin_key);
        debug_assert!(self.is_current_effect_scope(&plugin_key, &effect_scope));
        Arc::new(ScopedHostClient {
            handle: Arc::clone(self),
            plugin_id,
            plugin_key,
            effect_scope,
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
                this.handle_call_for_plugin(plugin_id.to_string().as_str(), &method, params)
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
        let callback_context = self
            .validated_callback_context(plugin_id.clone(), callback_context_from_params(&params))?;
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
        host_api::run_in_host_callback_context(callback_context, async {
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
                        scoped_context(plugin_id, None),
                        inner.read_config(p.path),
                    )
                    .await
                }
                method::HOST_CONFIG_RELOAD => {
                    let _p: HostConfigReloadParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.reload_config(),
                    )
                    .await?;
                    serde_json::to_value(out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_TOOL_INVOKE => {
                    let p: HostInvokeToolParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.invoke_tool(p.tool, p.input),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SERVICE_INVOKE => {
                    let p: HostInvokeServiceParams = parse(params)?;
                    let consumer = plugin_id.as_deref().ok_or_else(|| {
                        PluginError::invalid_params(
                            "service invocation requires an attributed consumer plugin",
                        )
                    })?;
                    let out = self
                        .invoke_service_for_plugin(
                            consumer,
                            p.request,
                            host_api::current_host_callback_context(),
                        )
                        .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_ASK_USER => {
                    let p: HostAskUserParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.ask_user(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SUBTASK_RUN => {
                    let p: HostRunSubtaskParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.run_subtask(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SUBTASK_CANCEL => {
                    let p: HostCancelSubtaskParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.cancel_subtask(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SUBTASK_MESSAGE => {
                    let p: HostMessageSubtaskParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.message_subtask(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SUBTASK_OUTPUT => {
                    let p: HostReadSubtaskOutputParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.read_subtask_output(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_TOOL_LIST => {
                    let _p: HostListToolsParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.list_tools(),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_CONTEXT_STATUS => {
                    let p: HostContextStatusParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.get_context_status(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SESSION_SET_MODEL => {
                    let p: HostSetSessionModelParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.set_session_model(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_IMAGE_EXECUTE => {
                    let p: HostImageExecuteParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.image_execute(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SNAPSHOT_ENTER => {
                    let p: HostEnterSnapshotParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.enter_snapshot(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SNAPSHOT_EXIT => {
                    let p: HostExitSnapshotParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.exit_snapshot(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_MONITOR_START => {
                    let p: HostMonitorStartParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.monitor_start(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_MONITOR_LIST => {
                    let _p: HostMonitorListParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.monitor_list(),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_MONITOR_READ => {
                    let p: HostMonitorReadParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.monitor_read(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_MONITOR_STOP => {
                    let p: HostMonitorStopParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
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
                        scoped_context(plugin_id, None),
                        inner.storage_get(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_STORAGE_SET => {
                    let p: HostStorageSetParams = parse(params)?;
                    host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.storage_set(p.request),
                    )
                    .await?;
                    Ok(serde_json::Value::Object(Default::default()))
                }
                method::HOST_STORAGE_DELETE => {
                    let p: HostStorageDeleteParams = parse(params)?;
                    host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.storage_delete(p.request),
                    )
                    .await?;
                    Ok(serde_json::Value::Object(Default::default()))
                }
                method::HOST_STORAGE_LIST => {
                    let p: HostStorageListParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.storage_list(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SECRET_GET => {
                    let p: HostSecretGetParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.secret_get(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SECRET_SET => {
                    let p: HostSecretSetParams = parse(params)?;
                    host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.secret_set(p.request),
                    )
                    .await?;
                    Ok(serde_json::Value::Object(Default::default()))
                }
                method::HOST_SECRET_DELETE => {
                    let p: HostSecretDeleteParams = parse(params)?;
                    host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.secret_delete(p.request),
                    )
                    .await?;
                    Ok(serde_json::Value::Object(Default::default()))
                }
                method::HOST_SECRET_LIST => {
                    let _p: HostSecretListParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
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
                    let _p: HostLspListServersParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.lsp_list_servers(),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_LSP_LIST_DIAGNOSTICS => {
                    let p: HostLspListDiagnosticsParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.lsp_list_diagnostics(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SNAPSHOT_LIST => {
                    let _p: HostSnapshotListParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.snapshot_list(),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SCHEDULER_LIST => {
                    let _p: HostSchedulerListParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.scheduler_list(),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SCHEDULER_CREATE => {
                    let p: HostSchedulerCreateParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.scheduler_create(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_SCHEDULER_DELETE => {
                    let p: HostSchedulerDeleteParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
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
                    let _p: HostMcpListServersParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.mcp_list_servers(),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_MCP_ADD_SERVER => {
                    let p: HostMcpAddServerParams = parse(params)?;
                    host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.mcp_add_server(p.request),
                    )
                    .await?;
                    Ok(serde_json::Value::Object(Default::default()))
                }
                method::HOST_MCP_REMOVE_SERVER => {
                    let p: HostMcpRemoveServerParams = parse(params)?;
                    let out = host_api::run_in_host_callback_context(
                        scoped_context(plugin_id, None),
                        inner.mcp_remove_server(p.request),
                    )
                    .await?;
                    serde_json::to_value(&out)
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_UI_DISPLAY_CONTRIBUTE => {
                    let plugin_id = plugin_id.ok_or_else(|| {
                        host_unavailable("ui.display.contribute requires plugin id")
                    })?;
                    let p: HostDisplayContributeParams = parse(params)?;
                    self.display_contribute(&plugin_id, p.request);
                    Ok(serde_json::Value::Object(Default::default()))
                }
                method::HOST_UI_DISPLAY_REMOVE => {
                    let plugin_id = plugin_id
                        .ok_or_else(|| host_unavailable("ui.display.remove requires plugin id"))?;
                    let p: HostDisplayRemoveParams = parse(params)?;
                    let removed = self.display_remove(&plugin_id, &p.request.contribution_id);
                    serde_json::to_value(&HostDisplayRemoveResponse { removed })
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                method::HOST_NOTIFY => {
                    let plugin_id =
                        plugin_id.ok_or_else(|| host_unavailable("notify requires plugin id"))?;
                    let p: HostNotifyParams = parse(params)?;
                    self.push_host_notification(&plugin_id, p.request);
                    Ok(serde_json::Value::Object(Default::default()))
                }
                method::HOST_UI_THEME_REGISTER => {
                    let plugin_id = plugin_id
                        .ok_or_else(|| host_unavailable("ui.theme.register requires plugin id"))?;
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
                    let plugin_id = plugin_id
                        .ok_or_else(|| host_unavailable("ui.theme.remove requires plugin id"))?;
                    let p: HostThemeRemoveParams = parse(params)?;
                    let removed = self.theme_remove(&plugin_id, &p.request.id);
                    serde_json::to_value(&HostThemeRemoveResponse { removed })
                        .map_err(|e| PluginError::invalid_params(e.to_string()))
                }
                other => Err(PluginError::not_implemented(other)),
            }
        })
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
        let tool = RegisteredTool::new(plugin_key.clone(), definition)
            .map_err(PluginError::invalid_params)?;
        let tool_key = tool.tool_key().clone();
        let plugin_tool_name = tool.tool_name().to_string();
        let scope = current_tool_scope();

        if let Some(scope) = scope {
            let owner = self.effect_scope(&plugin_key).ok_or_else(|| {
                host_unavailable(format!("plugin `{plugin_id}` has no active effect scope"))
            })?;
            let existed = self.scoped_tools.resolve(Some(&scope), &tool_key).is_some()
                || self
                    .tool_registry
                    .read()
                    .map_err(|_| host_unavailable("tool registry lock poisoned"))?
                    .lookup_tool_by_key(&tool_key)
                    .is_some();
            self.scoped_tools
                .replace_owned(
                    &owner,
                    Some(scope.clone()),
                    tool_key.clone(),
                    tool.clone(),
                    "host.tool",
                    format!("{scope}:{plugin_tool_name}"),
                )
                .map_err(|error| PluginError::internal(error.to_string()))?;
            let generation = self
                .tool_registry
                .write()
                .map_err(|_| host_unavailable("tool registry lock poisoned"))?
                .touch_generation();
            let event = ToolRegistryChangedEvent {
                kind: if existed {
                    ToolRegistryChangeKind::Updated
                } else {
                    ToolRegistryChangeKind::Registered
                },
                generation,
                timestamp_ms: unix_timestamp_ms(),
                plugin: plugin_key,
                tool_key,
                scope: Some(scope.to_string()),
                tool: Some(tool.definition.clone()),
            };
            self.record_tool_registry_event(event.clone());
            return Ok(HostToolMutationResponse {
                generation,
                model_name: Some(tool.canonical_name()),
                tool: Some(tool.definition),
                event: Some(event),
            });
        }

        let mut tool_registry = self
            .tool_registry
            .write()
            .map_err(|_| host_unavailable("tool registry lock poisoned"))?;
        let kind = if tool_registry.lookup_tool_by_key(&tool_key).is_some() {
            ToolRegistryChangeKind::Updated
        } else {
            ToolRegistryChangeKind::Registered
        };
        let tool = tool_registry
            .upsert_from_plugin(&plugin_key, tool.definition)
            .map_err(PluginError::invalid_params)?;
        let event = ToolRegistryChangedEvent {
            kind,
            generation: tool_registry.generation(),
            timestamp_ms: unix_timestamp_ms(),
            plugin: tool.plugin_key().clone(),
            tool_key: tool.tool_key().clone(),
            scope: None,
            tool: Some(tool.definition.clone()),
        };
        self.record_tool_registry_event(event.clone());
        let registry = Arc::clone(&self.tool_registry);
        let owned_plugin = plugin_key.clone();
        let owned_name = plugin_tool_name.clone();
        self.replace_effect_sync(&plugin_key, "host.tool", plugin_tool_name, move || {
            registry
                .write()
                .map_err(|_| "tool registry lock poisoned".to_string())?
                .remove_from_plugin(&owned_plugin, owned_name.as_str());
            Ok(())
        })?;
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
        let plugin_key: PluginKey = plugin_id.parse()?;
        let tool_key = if by_model_name {
            let tool_key: ToolKey = name.parse()?;
            if tool_key.plugin() != &plugin_key {
                return Err(host_unavailable(format!(
                    "tool `{name}` does not belong to plugin `{plugin_id}`"
                )));
            }
            tool_key
        } else {
            ToolKey::new(plugin_key.clone(), name.to_string())?
        };
        let tool_name = tool_key.name().to_string();

        if let Some(scope) = current_tool_scope() {
            let owner = self.effect_scope(&plugin_key).ok_or_else(|| {
                host_unavailable(format!("plugin `{plugin_id}` has no active effect scope"))
            })?;
            let removed = self
                .scoped_tools
                .remove_owned(&owner, Some(&scope), &tool_key)
                .map(|entry| entry.value);
            let fallback = self
                .tool_registry
                .read()
                .map_err(|_| host_unavailable("tool registry lock poisoned"))?
                .lookup_tool_by_key(&tool_key)
                .cloned();
            let generation = if removed.is_some() {
                self.tool_registry
                    .write()
                    .map_err(|_| host_unavailable("tool registry lock poisoned"))?
                    .touch_generation()
            } else {
                self.tool_registry
                    .read()
                    .map_err(|_| host_unavailable("tool registry lock poisoned"))?
                    .generation()
            };
            let event = removed.as_ref().map(|tool| ToolRegistryChangedEvent {
                kind: if fallback.is_some() {
                    ToolRegistryChangeKind::Updated
                } else {
                    ToolRegistryChangeKind::Removed
                },
                generation,
                timestamp_ms: unix_timestamp_ms(),
                plugin: tool.plugin_key().clone(),
                tool_key: tool.tool_key().clone(),
                scope: Some(scope.to_string()),
                tool: Some(fallback.as_ref().unwrap_or(tool).definition.clone()),
            });
            if let Some(event) = event.as_ref() {
                self.record_tool_registry_event(event.clone());
            }
            return Ok(HostToolMutationResponse {
                generation,
                model_name: removed.as_ref().map(RegisteredTool::canonical_name),
                tool: removed.map(|tool| tool.definition),
                event,
            });
        }

        let mut tool_registry = self
            .tool_registry
            .write()
            .map_err(|_| host_unavailable("tool registry lock poisoned"))?;
        let removed = tool_registry.remove_from_plugin(&plugin_key, tool_name.as_str());
        let event = removed.as_ref().map(|tool| ToolRegistryChangedEvent {
            kind: ToolRegistryChangeKind::Removed,
            generation: tool_registry.generation(),
            timestamp_ms: unix_timestamp_ms(),
            plugin: tool.plugin_key().clone(),
            tool_key: tool.tool_key().clone(),
            scope: None,
            tool: Some(tool.definition.clone()),
        });
        if let Some(event) = event.as_ref() {
            self.record_tool_registry_event(event.clone());
        }
        if removed.is_some() {
            self.release_effect(&plugin_key, "host.tool", tool_name.as_str());
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
        let scope = current_tool_scope();
        let registry = self
            .tool_registry
            .read()
            .map_err(|_| host_unavailable("tool registry lock poisoned"))?;
        let generation = registry.generation();
        let mut visible = registry
            .registered_tools()
            .map(|tool| (tool.tool_key().clone(), tool.clone()))
            .collect::<BTreeMap<_, _>>();
        drop(registry);
        for (key, entry) in self.scoped_tools.visible(scope.as_ref()) {
            visible.insert(key, entry.value);
        }
        let tools = visible
            .into_values()
            .map(|tool| HostRegisteredToolDescriptor {
                plugin: tool.plugin_key().clone(),
                tool_key: tool.tool_key().clone(),
                tool: tool.definition.clone(),
            })
            .collect();
        Ok(HostRegisteredToolListResponse {
            generation,
            tools,
            last_event: self.latest_visible_tool_registry_event(scope.as_ref()),
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
            for key in names.keys() {
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

    pub(super) fn display_contribute(&self, plugin_id: &str, req: HostDisplayContributeRequest) {
        let Ok(plugin_id) = plugin_id.parse::<PluginKey>() else {
            return;
        };
        let contribution_id = req.contribution.id.clone();
        if let Ok(mut guard) = self.display.write() {
            let key = (plugin_id.clone(), req.contribution.id.clone());
            guard.insert(
                key,
                HostDisplayContribution {
                    plugin_id: plugin_id.clone(),
                    contribution: req.contribution,
                },
            );
        }
        let display = Arc::clone(&self.display);
        let owned_plugin = plugin_id.clone();
        let owned_id = contribution_id.clone();
        if let Err(error) =
            self.replace_effect_sync(&plugin_id, "host.display", contribution_id, move || {
                display
                    .write()
                    .map_err(|_| "display registry lock poisoned".to_string())?
                    .remove(&(owned_plugin, owned_id));
                Ok(())
            })
        {
            self.logs.append(
                &plugin_id,
                "error",
                "effects",
                format!("failed to own display contribution: {error}"),
                serde_json::Value::Null,
            );
        }
    }

    pub(super) fn display_remove(&self, plugin_id: &str, contribution_id: &str) -> bool {
        let Ok(plugin_id) = plugin_id.parse::<PluginKey>() else {
            return false;
        };
        if let Ok(mut guard) = self.display.write() {
            let removed = guard
                .remove(&(plugin_id.clone(), contribution_id.to_string()))
                .is_some();
            if removed {
                self.release_effect(&plugin_id, "host.display", contribution_id);
            }
            return removed;
        }
        false
    }

    /// Record a plugin notification intent through the unified `host.notify`
    /// entry. The queue is bounded: oldest entries drop first so a frontend
    /// that polls late never observes unbounded growth.
    pub(super) fn push_host_notification(&self, plugin_id: &str, req: PluginNotifyRequest) {
        const HOST_NOTIFICATION_QUEUE_LIMIT: usize = 64;
        let Ok(plugin_id) = plugin_id.parse::<PluginKey>() else {
            return;
        };
        if let Ok(mut guard) = self.host_notifications.write() {
            guard.push_back(HostNotification {
                plugin_id: plugin_id.to_string(),
                title: req.title,
                body: req.body,
                severity: req.severity,
                session_id: req.session_id,
                actions: req.actions,
            });
            while guard.len() > HOST_NOTIFICATION_QUEUE_LIMIT {
                guard.pop_front();
            }
        }
    }

    /// Snapshot of the recent plugin notifications (newest last). Frontends
    /// dedupe/consume by `plugin_id:severity:body`.
    pub fn host_notifications(&self) -> Vec<HostNotification> {
        self.host_notifications
            .read()
            .map(|guard| guard.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn display_list_response(&self) -> Vec<HostDisplayContribution> {
        let mut contributions: Vec<HostDisplayContribution> = self
            .display
            .read()
            .map(|guard| guard.values().cloned().collect())
            .unwrap_or_default();
        contributions.sort_by(|a, b| {
            b.contribution
                .priority
                .cmp(&a.contribution.priority)
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
                .then_with(|| a.contribution.id.cmp(&b.contribution.id))
        });
        contributions
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
        let theme_id = req.id.clone();
        let mut guard = self
            .themes
            .write()
            .map_err(|_| host_unavailable("theme registry lock poisoned"))?;
        let key = (plugin_id.clone(), theme_id.clone());
        guard.insert(
            key,
            HostThemePalette {
                id: theme_id.clone(),
                plugin_id: plugin_id.clone(),
                display_name: req.display_name,
                colors: req.colors,
            },
        );
        drop(guard);
        let themes = Arc::clone(&self.themes);
        let owned_plugin = plugin_id.clone();
        let owned_id = theme_id.clone();
        self.replace_effect_sync(&plugin_id, "host.theme", theme_id, move || {
            themes
                .write()
                .map_err(|_| "theme registry lock poisoned".to_string())?
                .remove(&(owned_plugin, owned_id));
            Ok(())
        })?;
        Ok(())
    }

    pub(super) fn theme_remove(&self, plugin_id: &str, id: &str) -> bool {
        let Ok(plugin_id) = plugin_id.parse::<PluginKey>() else {
            return false;
        };
        if let Ok(mut guard) = self.themes.write() {
            let removed = guard.remove(&(plugin_id.clone(), id.to_owned())).is_some();
            if removed {
                self.release_effect(&plugin_id, "host.theme", id);
            }
            return removed;
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
    Arc, BTreeMap, CallbackAuthorityRecord, EventEnvelope, EventSubscription, HashMap,
    HostAskUserParams, HostCallbackContext, HostCancelSubtaskParams, HostClient,
    HostConfigReadParams, HostConfigReloadParams, HostContextStatusParams,
    HostDisplayContributeParams, HostDisplayContributeRequest, HostDisplayContribution,
    HostDisplayRemoveParams, HostDisplayRemoveResponse, HostEnterSnapshotParams,
    HostExitSnapshotParams, HostHandle, HostHookListResponse, HostHookRegistration,
    HostImageExecuteParams, HostInvokeServiceParams, HostInvokeToolParams, HostListToolsParams,
    HostLogParams, HostLspListDiagnosticsParams, HostLspListServersParams, HostMcpAddServerParams,
    HostMcpListServersParams, HostMcpRemoveServerParams, HostMessageSubtaskParams,
    HostMonitorListParams, HostMonitorReadParams, HostMonitorStartParams, HostMonitorStopParams,
    HostNotification, HostNotifyParams, HostPluginStatusGetParams, HostPluginStatusGetResponse,
    HostPluginStatusListResponse, HostReadSubtaskOutputParams, HostRegisteredToolDescriptor,
    HostRegisteredToolListResponse, HostRunSubtaskParams, HostSchedulerCreateParams,
    HostSchedulerDeleteParams, HostSchedulerListParams, HostSecretDeleteParams,
    HostSecretGetParams, HostSecretListParams, HostSecretSetParams, HostSetSessionModelParams,
    HostSnapshotListParams, HostStorageDeleteParams, HostStorageGetParams, HostStorageListParams,
    HostStorageSetParams, HostSubscribeParams, HostThemeListResponse, HostThemePalette,
    HostThemeRegisterParams, HostThemeRegisterRequest, HostThemeRemoveParams,
    HostThemeRemoveResponse, HostToolMutationResponse, HostToolRegisterParams,
    HostToolRemoveParams, HostToolUpdateParams, HostUnsubscribeParams, Mutex, PluginEffectScope,
    PluginEffectScopeInspect, PluginError, PluginErrorKind, PluginKey, PluginLogRecord,
    PluginLogStore, PluginNotifyRequest, PluginOperationCatalogItem, PluginScopeKey,
    PluginServiceBinding, PluginServiceBindingKey, PluginServiceInvokeInput,
    PluginServiceInvokeOutput, PluginToolRegistry, PluginTransport, RegisteredTool, RwLock,
    ScopedHostClient, ScopedRegistry, ToolKey, ToolRegistryChangeKind, ToolRegistryChangedEvent,
    ToolRegistryEventListener, VecDeque, callback_context_from_params, current_tool_scope,
    host_api, host_status_from, host_unavailable, method, operation_registry_name, parse,
    scoped_context, tool_registry_event_visible_in_scope, transport_to_plugin_error,
    unix_timestamp_ms,
};

#[cfg(test)]
mod effect_ownership_tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::PluginScopeKey;

    fn test_handle(
        plugin_id: &PluginKey,
    ) -> (
        HostHandle,
        Arc<RwLock<PluginToolRegistry>>,
        Arc<RwLock<HashMap<PluginKey, usize>>>,
    ) {
        let tools = Arc::new(RwLock::new(PluginToolRegistry::new()));
        let indices = Arc::new(RwLock::new(HashMap::from([(plugin_id.clone(), 0)])));
        let statuses = Arc::new(crate::status::StatusRegistry::new());
        let logs = Arc::new(PluginLogStore::default());
        let handle = HostHandle::new_with_components(
            Arc::new(crate::sdk::host_api::NoopHostClient),
            Arc::clone(&tools),
            Arc::clone(&indices),
            Arc::new(RwLock::new(HashMap::new())),
            statuses,
            logs,
            None,
        );
        (handle, tools, indices)
    }

    fn tool(name: &str) -> crate::sdk::ToolDefinition {
        crate::sdk::ToolDefinition {
            name: name.to_string(),
            contract: crate::sdk::ToolContract {
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false,
                }),
                ..Default::default()
            },
            model: Default::default(),
            docs: crate::sdk::ToolDocs {
                summary: Some("Dynamic effect-owned tool.".to_string()),
                ..Default::default()
            },
            runtime: Default::default(),
            permissions: Default::default(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn callback_authority_rejects_forgery_cross_plugin_replay_and_stale_generations() {
        let plugin_id: PluginKey = "example.authority".parse().expect("plugin key");
        let other_id: PluginKey = "example.other".parse().expect("other plugin key");
        let (handle, _tools, _indices) = test_handle(&plugin_id);
        let first_scope = handle.begin_plugin_instance(plugin_id.clone());

        let forged = HostCallbackContext {
            session_id: Some(7),
            call_id: Some(11),
            workspace_root: Some("/trusted/workspace".to_string()),
            tool_name: Some("write".to_string()),
            ..Default::default()
        };
        let error = handle
            .validated_callback_context(Some(plugin_id.to_string()), Some(forged.clone()))
            .expect_err("privileged context without token must be rejected");
        assert_eq!(error.kind, PluginErrorKind::PolicyDenied);

        let (issued, lease) = handle.issue_callback_authority(&plugin_id, forged.clone());
        let trusted = handle
            .validated_callback_context(Some(plugin_id.to_string()), Some(issued.clone()))
            .expect("live authority token");
        assert_eq!(trusted.session_id, Some(7));
        assert_eq!(trusted.call_id, Some(11));
        assert_eq!(trusted.tool_name.as_deref(), Some("write"));

        let cross_plugin = handle
            .validated_callback_context(Some(other_id.to_string()), Some(issued.clone()))
            .expect_err("authority token must be bound to one plugin");
        assert_eq!(cross_plugin.kind, PluginErrorKind::PolicyDenied);

        let mut tampered = issued.clone();
        tampered.tool_name = Some("admin".to_string());
        let tampered = handle
            .validated_callback_context(Some(plugin_id.to_string()), Some(tampered))
            .expect_err("authority context fields are echo-only");
        assert_eq!(tampered.kind, PluginErrorKind::PolicyDenied);

        let second_scope = handle.begin_plugin_instance(plugin_id.clone());
        assert_ne!(first_scope.generation(), second_scope.generation());
        let stale = handle
            .validated_callback_context(Some(plugin_id.to_string()), Some(issued.clone()))
            .expect_err("authority from an older plugin generation must be rejected");
        assert_eq!(stale.kind, PluginErrorKind::PolicyDenied);

        drop(lease);
        let expired = handle
            .validated_callback_context(Some(plugin_id.to_string()), Some(issued))
            .expect_err("revoked authority token must not replay");
        assert_eq!(expired.kind, PluginErrorKind::PolicyDenied);
    }

    #[tokio::test]
    async fn dynamic_tools_are_visible_only_in_the_registration_session_scope() {
        let plugin_id: PluginKey = "example.scoped-tools".parse().expect("plugin key");
        let (handle, tools, _indices) = test_handle(&plugin_id);
        handle.begin_plugin_instance(plugin_id.clone());

        let register = handle.run_in_authorized_callback_context(
            &plugin_id,
            HostCallbackContext {
                session_id: Some(41),
                ..Default::default()
            },
            async {
                handle.tool_upsert_for_plugin(plugin_id.to_string().as_str(), tool("dynamic"))
            },
        );
        let registered = register.await.expect("session-scoped tool registration");
        assert_eq!(
            registered
                .event
                .as_ref()
                .and_then(|event| event.scope.as_deref()),
            Some("session:41")
        );
        assert!(
            tools
                .read()
                .expect("global tool registry")
                .lookup_for_plugin(&plugin_id, "dynamic")
                .is_none(),
            "session registration must not mutate the global registry"
        );

        let session_41 = handle
            .run_in_authorized_callback_context(
                &plugin_id,
                HostCallbackContext {
                    session_id: Some(41),
                    ..Default::default()
                },
                async { handle.registered_tool_list_response() },
            )
            .await
            .expect("session 41 catalog");
        assert_eq!(session_41.tools.len(), 1);
        assert_eq!(session_41.tools[0].tool.name, "dynamic");
        assert_eq!(
            session_41
                .last_event
                .as_ref()
                .and_then(|event| event.scope.as_deref()),
            Some("session:41")
        );

        let session_42 = handle
            .run_in_authorized_callback_context(
                &plugin_id,
                HostCallbackContext {
                    session_id: Some(42),
                    ..Default::default()
                },
                async { handle.registered_tool_list_response() },
            )
            .await
            .expect("session 42 catalog");
        assert!(session_42.tools.is_empty());
        assert!(session_42.last_event.is_none());
        assert!(
            handle
                .registered_tool_list_response()
                .expect("global catalog")
                .tools
                .is_empty()
        );
        assert!(
            handle
                .registered_tool_list_response()
                .expect("global catalog")
                .last_event
                .is_none(),
            "global callers must not observe another session's scoped event payload"
        );

        let session_42_remove = handle
            .run_in_authorized_callback_context(
                &plugin_id,
                HostCallbackContext {
                    session_id: Some(42),
                    ..Default::default()
                },
                async {
                    handle.tool_remove_for_plugin(plugin_id.to_string().as_str(), "dynamic", false)
                },
            )
            .await
            .expect("other-session remove is a no-op");
        assert!(session_42_remove.tool.is_none());

        let removed = handle
            .run_in_authorized_callback_context(
                &plugin_id,
                HostCallbackContext {
                    session_id: Some(41),
                    ..Default::default()
                },
                async {
                    handle.tool_remove_for_plugin(plugin_id.to_string().as_str(), "dynamic", false)
                },
            )
            .await
            .expect("own-session removal");
        assert_eq!(
            removed
                .event
                .as_ref()
                .and_then(|event| event.scope.as_deref()),
            Some("session:41")
        );
        assert!(
            handle
                .scoped_tool_registry()
                .resolve(
                    Some(&PluginScopeKey::session(41)),
                    &"example.scoped-tools.dynamic".parse().unwrap()
                )
                .is_none()
        );
    }

    #[tokio::test]
    async fn removing_a_session_tool_shadow_restores_global_visibility_as_an_update() {
        let plugin_id: PluginKey = "example.scoped-shadow".parse().expect("plugin key");
        let (handle, _tools, _indices) = test_handle(&plugin_id);
        handle.begin_plugin_instance(plugin_id.clone());
        let plugin_id_text = plugin_id.to_string();

        let mut global = tool("dynamic");
        global.docs.summary = Some("global".to_string());
        handle
            .tool_upsert_for_plugin(plugin_id_text.as_str(), global)
            .expect("global dynamic tool");

        let mut scoped = tool("dynamic");
        scoped.docs.summary = Some("session".to_string());
        handle
            .run_in_authorized_callback_context(
                &plugin_id,
                HostCallbackContext {
                    session_id: Some(77),
                    ..Default::default()
                },
                async { handle.tool_upsert_for_plugin(plugin_id_text.as_str(), scoped) },
            )
            .await
            .expect("session shadow");

        let visible = handle
            .run_in_authorized_callback_context(
                &plugin_id,
                HostCallbackContext {
                    session_id: Some(77),
                    ..Default::default()
                },
                async { handle.registered_tool_list_response() },
            )
            .await
            .expect("session catalog");
        assert_eq!(
            visible.tools[0].tool.docs.summary.as_deref(),
            Some("session")
        );

        let removed = handle
            .run_in_authorized_callback_context(
                &plugin_id,
                HostCallbackContext {
                    session_id: Some(77),
                    ..Default::default()
                },
                async { handle.tool_remove_for_plugin(plugin_id_text.as_str(), "dynamic", false) },
            )
            .await
            .expect("remove session shadow");
        let event = removed.event.expect("visibility change event");
        assert_eq!(event.kind, ToolRegistryChangeKind::Updated);
        assert_eq!(event.scope.as_deref(), Some("session:77"));
        assert_eq!(
            event
                .tool
                .as_ref()
                .and_then(|tool| tool.docs.summary.as_deref()),
            Some("global")
        );

        let visible = handle
            .run_in_authorized_callback_context(
                &plugin_id,
                HostCallbackContext {
                    session_id: Some(77),
                    ..Default::default()
                },
                async { handle.registered_tool_list_response() },
            )
            .await
            .expect("session catalog after fallback");
        assert_eq!(visible.tools.len(), 1);
        assert_eq!(
            visible.tools[0].tool.docs.summary.as_deref(),
            Some("global")
        );
    }

    #[tokio::test]
    async fn dynamic_host_contributions_share_one_effect_scope_and_dispose_cleanly() {
        let plugin_id: PluginKey = "example.effects".parse().expect("plugin key");
        let (handle, tools, _indices) = test_handle(&plugin_id);
        let plugin_id_text = plugin_id.to_string();

        handle
            .tool_upsert_for_plugin(plugin_id_text.as_str(), tool("dynamic"))
            .expect("register dynamic tool");
        handle.display_contribute(
            plugin_id_text.as_str(),
            HostDisplayContributeRequest {
                contribution: crate::sdk::PluginDisplayContribution {
                    id: "status".to_string(),
                    kind: crate::sdk::ContributionKind::StatusLineText,
                    priority: 0,
                    content: crate::sdk::PluginDisplayContent::Text {
                        text: "ready".to_string(),
                    },
                },
            },
        );
        handle
            .theme_register(
                plugin_id_text.as_str(),
                HostThemeRegisterRequest {
                    id: "paper".to_string(),
                    display_name: "Paper".to_string(),
                    colors: Default::default(),
                },
            )
            .expect("register theme");

        let inspect = handle
            .effect_scope_inspect(&plugin_id)
            .expect("effect scope inspect");
        assert_eq!(
            inspect.lifecycle,
            crate::effect_scope::PluginEffectScopeState::Active
        );
        assert_eq!(
            inspect
                .effects
                .iter()
                .map(|effect| effect.kind.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["host.display", "host.theme", "host.tool"])
        );
        assert!(
            tools
                .read()
                .expect("tool registry")
                .lookup_for_plugin(&plugin_id, "dynamic")
                .is_some()
        );
        assert_eq!(handle.display_list_response().len(), 1);
        assert_eq!(handle.theme_list_response().themes.len(), 1);

        handle.dispose_plugin_resources(&plugin_id).await;

        assert!(
            tools
                .read()
                .expect("tool registry")
                .lookup_for_plugin(&plugin_id, "dynamic")
                .is_none()
        );
        assert!(handle.display_list_response().is_empty());
        assert!(handle.theme_list_response().themes.is_empty());
        let disposed = handle
            .effect_scope_inspect(&plugin_id)
            .expect("disposed effect inspect");
        assert_eq!(
            disposed.lifecycle,
            crate::effect_scope::PluginEffectScopeState::Disposed
        );
        assert!(
            disposed
                .effects
                .iter()
                .all(|effect| { effect.state == crate::effect_scope::PluginEffectState::Disposed })
        );
    }

    #[tokio::test]
    async fn manifest_operations_are_exact_effect_owned_registry_entries() {
        use crate::sdk::{
            OperationDiscoverability, PluginManifest, PluginOperationDefinition,
            PluginOperationTarget, SettingsContract, SettingsNode,
        };

        let plugin_id: PluginKey = "example.operations".parse().expect("plugin key");
        let (handle, _tools, _indices) = test_handle(&plugin_id);
        let mut manifest = PluginManifest::new("example", "operations", "0.1.0");
        manifest.operations.push(PluginOperationDefinition {
            id: "open".to_string(),
            title: "Open".to_string(),
            description: "Open the plugin workbench.".to_string(),
            group: "Plugin".to_string(),
            category: None,
            slash: Some("open".to_string()),
            aliases: Vec::new(),
            usage: None,
            input: SettingsContract::new(SettingsNode::root_object("Input", "")),
            discoverability: OperationDiscoverability::default(),
            target: PluginOperationTarget::Method {
                handler: "open".to_string(),
            },
        });

        handle
            .own_manifest_resources(&plugin_id, &manifest)
            .expect("own manifest operation");
        let registry = handle.operation_registry();
        let name = operation_registry_name(&plugin_id, "open");
        {
            let entry = registry.resolve(None, &name).expect("registered operation");
            assert_eq!(entry.owner, plugin_id);
            assert_eq!(entry.value.operation.id, "open");
        }
        let inspect = handle
            .effect_scope_inspect(&plugin_id)
            .expect("effect scope inspect");
        assert!(inspect.effects.iter().any(|effect| {
            effect.kind == "host.operation"
                && effect.label == "open"
                && effect.state == crate::effect_scope::PluginEffectState::Active
        }));

        handle.dispose_plugin_resources(&plugin_id).await;

        assert!(registry.resolve(None, &name).is_none());
        let disposed = handle
            .effect_scope_inspect(&plugin_id)
            .expect("disposed scope");
        assert!(disposed.effects.iter().any(|effect| {
            effect.kind == "host.operation"
                && effect.label == "open"
                && effect.state == crate::effect_scope::PluginEffectState::Disposed
        }));
    }

    #[tokio::test]
    async fn explicit_remove_releases_owned_effect_without_double_cleanup() {
        let plugin_id: PluginKey = "example.effects".parse().expect("plugin key");
        let (handle, _tools, _indices) = test_handle(&plugin_id);
        let plugin_id_text = plugin_id.to_string();
        handle
            .theme_register(
                plugin_id_text.as_str(),
                HostThemeRegisterRequest {
                    id: "paper".to_string(),
                    display_name: "Paper".to_string(),
                    colors: Default::default(),
                },
            )
            .expect("register theme");
        assert!(handle.theme_remove(plugin_id_text.as_str(), "paper"));
        let inspect = handle
            .effect_scope_inspect(&plugin_id)
            .expect("effect inspect");
        assert_eq!(
            inspect.effects[0].state,
            crate::effect_scope::PluginEffectState::Disposed
        );
        handle.dispose_plugin_resources(&plugin_id).await;
        assert!(handle.theme_list_response().themes.is_empty());
    }
}
