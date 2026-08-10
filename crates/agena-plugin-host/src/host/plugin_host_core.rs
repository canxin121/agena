impl PluginHost {
    pub fn new_empty() -> Arc<Self> {
        let tool_registry = Arc::new(RwLock::new(PluginToolRegistry::new()));
        let statuses = Arc::new(crate::status::StatusRegistry::new());
        let logs = Arc::new(PluginLogStore::default());
        let host_handle = Arc::new(HostHandle::new_with_components(
            Arc::new(NoopHostClient),
            Arc::clone(&tool_registry),
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(RwLock::new(HashMap::new())),
            Arc::clone(&statuses),
            Arc::clone(&logs),
            None,
        ));
        Arc::new(Self {
            plugins: Vec::new(),
            plugins_by_id: HashMap::new(),
            tool_registry,
            statuses,
            logs,
            timeouts: TimeoutsConfig::default(),
            _host_handle: host_handle,
            transferred_to_successor: tokio::sync::Mutex::new(Default::default()),
            hook_runs: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn plugins(&self) -> &[Arc<LoadedPlugin>] {
        &self.plugins
    }

    pub fn plugin_summary(&self) -> (usize, BTreeMap<&'static str, usize>) {
        let mut by_kind: BTreeMap<&'static str, usize> = BTreeMap::new();
        for p in &self.plugins {
            *by_kind.entry(p.kind).or_insert(0) += 1;
        }
        (self.plugins.len(), by_kind)
    }

    pub fn lookup_tool(&self, canonical_name: &str) -> Option<RegisteredTool> {
        self.tool_registry
            .read()
            .ok()?
            .lookup_tool_by_canonical_name(canonical_name)
            .cloned()
    }

    pub fn registered_tools(&self) -> Vec<RegisteredTool> {
        self.tool_registry
            .read()
            .map(|reg| reg.registered_tools_owned())
            .unwrap_or_default()
    }

    pub fn tool_registry_snapshot(&self) -> crate::registry::ToolRegistrySnapshot {
        self.tool_registry
            .read()
            .map(|reg| reg.snapshot())
            .unwrap_or_else(|_| crate::registry::ToolRegistrySnapshot {
                generation: 0,
                tools: Vec::new(),
            })
    }

    pub fn tool_registry_generation(&self) -> u64 {
        self.tool_registry
            .read()
            .map(|reg| reg.generation())
            .unwrap_or(0)
    }

    pub fn status_registry(&self) -> Arc<crate::status::StatusRegistry> {
        Arc::clone(&self.statuses)
    }

    pub fn plugin_status(&self, plugin_id: &str) -> Option<crate::status::PluginStatus> {
        let plugin_key: PluginKey = plugin_id.parse().ok()?;
        self.statuses.get(&plugin_key)
    }

    pub fn plugin_status_by_key(
        &self,
        plugin_id: &PluginKey,
    ) -> Option<crate::status::PluginStatus> {
        self.statuses.get(plugin_id)
    }

    pub fn plugin_statuses(&self) -> Vec<crate::status::PluginStatus> {
        self.statuses.list()
    }

    pub fn log_store(&self) -> Arc<PluginLogStore> {
        Arc::clone(&self.logs)
    }

    pub fn tool_registry_events_since(
        &self,
        after_generation: Option<u64>,
        limit: usize,
    ) -> Vec<ToolRegistryChangedEvent> {
        self._host_handle
            .tool_registry_events_since(after_generation, limit)
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

    pub fn plugin_inspect(&self, plugin_id: &str) -> Option<PluginInspect> {
        let status = self.plugin_status(plugin_id)?;
        let plugin_key: PluginKey = plugin_id.parse().ok()?;
        let plugin = self.plugins_by_id.get(&plugin_key);
        let manifest = plugin.as_ref().map(|plugin| plugin.manifest.clone());
        let authority = plugin.map(|plugin| plugin.authority_summary());
        let configured_plugin = plugin
            .as_ref()
            .map(|plugin| plugin.configured_plugin.clone());
        let hooks = plugin
            .as_ref()
            .map(|plugin| vec![hook_registration_for_plugin(plugin)])
            .unwrap_or_default();
        Some(PluginInspect {
            status,
            manifest,
            authority,
            hooks,
            configured_plugin,
        })
    }

    /// Native asynchronous `tool.before` dispatch.
    pub async fn dispatch_tool_before(
        &self,
        input: ToolBeforeInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<ToolBeforeInput, PluginError> {
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let plugins = self.plugins.clone();
        let mut current = input;
        for plugin in &plugins {
            if !plugin.subscribes(HookSubscription::TOOL_BEFORE) {
                continue;
            }
            let params = serde_json::to_value(&current)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let context = tool_hook_context(
                plugin,
                current.tool_name(),
                Some(current.session_id),
                Some(current.call_id),
                Some(current.workspace_root.clone()),
            );
            let value = await_transport_with_cancellation(
                cancellation.clone(),
                host_api::run_in_host_callback_context(
                    context,
                    call_with_timeout(plugin, method::HOOK_TOOL_BEFORE, params, timeout),
                ),
            )
            .await
            .map_err(transport_to_plugin_error)?;
            if matches!(&value, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<ToolBeforePatch> = serde_json::from_value(value)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let Some(patch) = patch else {
                continue;
            };
            if let Some(reason) = patch.abort_reason {
                return Err(PluginError::internal(reason));
            }
            if let Some(v) = patch.input {
                current.input = v;
            }
            if let Some(t) = patch.title_override {
                current.title_override = Some(t);
            }
            for (k, v) in patch.metadata {
                current.metadata.insert(k, v);
            }
        }
        Ok(current)
    }

    /// Native asynchronous `tool.after` dispatch for runtime request paths.
    pub async fn dispatch_tool_after(
        &self,
        input: ToolAfterInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<ToolAfterInput, PluginError> {
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let plugins = self.plugins.clone();
        let session_id = Some(input.session_id);
        let mut runs = Vec::new();
        let res = await_transport_with_cancellation(
            cancellation,
            dispatcher::chain_patch_in_context::<ToolAfterInput, ToolAfterPatch, _, _>(
                &plugins,
                method::HOOK_TOOL_AFTER,
                HookSubscription::TOOL_AFTER,
                timeout,
                input,
                |inp, patch| {
                    if let Some(t) = patch.title {
                        inp.title = t;
                    }
                    if let Some(summary) = patch.summary {
                        inp.summary = summary;
                    }
                    if let Some(o) = patch.output_text {
                        inp.output_text = o;
                    }
                    if let Some(p) = patch.payload {
                        inp.payload = Some(p);
                    }
                    for (k, v) in patch.metadata {
                        inp.metadata.insert(k, v);
                    }
                },
                |plugin, input| {
                    Some(tool_hook_context(
                        plugin,
                        input.tool_name(),
                        Some(input.session_id),
                        Some(input.call_id),
                        Some(input.workspace_root.clone()),
                    ))
                },
                session_id,
                &mut runs,
            ),
        )
        .await;
        // tool.after activity is intentionally not recorded (not part of the
        // transcript hook-run scope); the runs are discarded.
        let _ = runs;
        res.map_err(transport_to_plugin_error)
    }

    /// Native asynchronous tool invocation. This is the canonical runtime
    /// entry point; cancellation and the per-tool deadline are polled by the
    /// same Tokio runtime that owns the transport.
    pub async fn invoke_tool(
        &self,
        registered_tool: &RegisteredTool,
        input: ToolInvokeInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(registered_tool.plugin_key())
            .cloned()
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "plugin `{}` not loaded",
                    registered_tool.plugin_full_name()
                ))
            })?;
        let timeout = self.tool_invoke_timeout(registered_tool);
        let mut input = input;
        // ensure tool name is the plugin-original name (in case caller passed model name)
        input.tool_name = registered_tool.tool_name().to_string();
        let session_id = input.session_id;
        let call_id = input.call_id;
        let workspace_root = input.workspace_root.clone();
        let plugin_id = registered_tool.plugin_full_name().clone();
        let tool_name = registered_tool.tool_name().to_string();
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let invoke = host_api::run_in_host_callback_context(
            HostCallbackContext {
                plugin_id: Some(plugin_id),
                session_id: Some(session_id),
                call_id: Some(call_id),
                workspace_root: Some(workspace_root),
                tool_name: Some(tool_name),
            },
            call_with_timeout(&plugin, method::HOOK_TOOL_INVOKE, params, timeout),
        );
        let result = match cancellation {
            Some(cancellation) => tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(TransportError::Cancelled),
                result = invoke => result,
            },
            None => invoke.await,
        };
        let value = result.map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    pub async fn invoke_plugin_command_async(
        &self,
        plugin_id: &str,
        input: PluginCommandInvokeInput,
    ) -> Result<PluginCommandOutput, PluginError> {
        let plugin_key: PluginKey = plugin_id
            .parse()
            .map_err(|err| PluginError::invalid_params(format!("invalid plugin id: {err}")))?;
        let plugin = self
            .plugins_by_id
            .get(&plugin_key)
            .cloned()
            .ok_or_else(|| {
                PluginError::not_implemented(format!("plugin `{plugin_id}` not loaded"))
            })?;
        if input.command_id.is_empty() {
            return Err(PluginError::invalid_params(
                "command_id must not be empty".to_string(),
            ));
        }
        let timeout = self.timeouts.fast_or(Duration::from_secs(10));
        let plugin_id = plugin_id.to_string();
        let session_id = input.session_id;
        let call_id = input.call_id;
        let workspace_root = input.workspace_root.clone();
        let command_id = input.command_id.clone();
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let result = host_api::run_in_host_callback_context(
            HostCallbackContext {
                plugin_id: Some(plugin_id),
                session_id,
                call_id,
                workspace_root,
                tool_name: None,
            },
            call_with_timeout(&plugin, method::COMMAND_INVOKE, params, timeout),
        )
        .await;
        let value = result.map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|e| {
            PluginError::invalid_params(format!("invalid command output for `{command_id}`: {e}"))
        })
    }

    pub(super) fn tool_invoke_timeout(&self, registered_tool: &RegisteredTool) -> Duration {
        let base = self.timeouts.tool_invoke_or(Duration::from_secs(300));
        // Interactive and subtask tools need long-lived budgets; these are
        // declared on the tool's permission contract, not static capabilities.
        let contract = &registered_tool.definition.permissions;
        if contract.interactive || contract.task {
            return base.max(Duration::from_secs(60 * 60 * 24));
        }
        base
    }

    pub async fn dispatch_tool_permission_paths(
        &self,
        registered_tool: &RegisteredTool,
        input: ToolPermissionPathsInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<Vec<crate::sdk::PathRequest>, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(registered_tool.plugin_key())
            .cloned()
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "plugin `{}` not loaded",
                    registered_tool.plugin_full_name()
                ))
            })?;
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let mut input = input;
        input.tool_name = registered_tool.tool_name().to_string();
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let plugin_id = registered_tool.plugin_full_name().clone();
        let tool_name = registered_tool.tool_name().to_string();
        let workspace_root = input.workspace_root.clone();
        let result = await_transport_with_cancellation(
            cancellation,
            host_api::run_in_host_callback_context(
                HostCallbackContext {
                    plugin_id: Some(plugin_id),
                    workspace_root: Some(workspace_root),
                    tool_name: Some(tool_name),
                    ..Default::default()
                },
                call_with_timeout(&plugin, method::HOOK_TOOL_PERMISSION_PATHS, params, timeout),
            ),
        )
        .await;
        let value = result.map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    pub async fn dispatch_tool_permission_networks(
        &self,
        registered_tool: &RegisteredTool,
        input: ToolPermissionNetworksInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<Vec<crate::sdk::NetworkRequest>, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(registered_tool.plugin_key())
            .cloned()
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "plugin `{}` not loaded",
                    registered_tool.plugin_full_name()
                ))
            })?;
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let mut input = input;
        input.tool_name = registered_tool.tool_name().to_string();
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let plugin_id = registered_tool.plugin_full_name().clone();
        let tool_name = registered_tool.tool_name().to_string();
        let workspace_root = input.workspace_root.clone();
        let result = await_transport_with_cancellation(
            cancellation,
            host_api::run_in_host_callback_context(
                HostCallbackContext {
                    plugin_id: Some(plugin_id),
                    workspace_root: Some(workspace_root),
                    tool_name: Some(tool_name),
                    ..Default::default()
                },
                call_with_timeout(
                    &plugin,
                    method::HOOK_TOOL_PERMISSION_NETWORKS,
                    params,
                    timeout,
                ),
            ),
        )
        .await;
        let value = result.map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    /// Streaming variant: returns a receiver of [`ToolStreamChunk`]s plus a
    /// oneshot for the terminal [`ToolStreamEnd`] (or error). Transports with
    /// native stream support should surface it through `PluginTransport`;
    /// others fall back to a single-chunk emulation built from the regular
    /// `tool_invoke` response.
    pub async fn invoke_tool_stream(
        &self,
        registered_tool: &RegisteredTool,
        input: ToolInvokeInput,
    ) -> Result<ToolInvokeStream, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(registered_tool.plugin_key())
            .cloned()
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "plugin `{}` not loaded",
                    registered_tool.plugin_full_name()
                ))
            })?;
        let mut input = input;
        input.tool_name = registered_tool.tool_name().to_string();

        let context = tool_hook_context(
            &plugin,
            &input.tool_name,
            Some(input.session_id),
            Some(input.call_id),
            Some(input.workspace_root.clone()),
        );
        if let Some(stream) = host_api::run_in_host_callback_context(
            context.clone(),
            plugin.transport.invoke_stream(input.clone()),
        )
        .await
        .map_err(transport_to_plugin_error)?
        {
            return Ok(ToolInvokeStream {
                stream_id: stream.stream_id,
                chunks: stream.chunks,
                end: stream.end,
            });
        }

        let timeout = self.tool_invoke_timeout(registered_tool);
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let invoke_result = host_api::run_in_host_callback_context(
            context,
            call_with_timeout(&plugin, method::HOOK_TOOL_INVOKE, params, timeout),
        )
        .await
        .map_err(transport_to_plugin_error)?;
        let result: ToolInvokeOutput = serde_json::from_value(invoke_result)
            .map_err(|e| PluginError::invalid_params(e.to_string()))?;

        let (tx, rx) = tokio::sync::mpsc::channel::<ToolStreamChunk>(8);
        let (end_tx, end_rx) = tokio::sync::oneshot::channel();
        let stream_id = format!("emu-{}", uuid::Uuid::new_v4().simple());
        let chunk = ToolStreamChunk {
            stream_id: stream_id.clone(),
            text_delta: Some(result.output_text.clone()),
            metadata: result.metadata.clone(),
        };
        let _ = tx.send(chunk).await;
        drop(tx);
        let _ = end_tx.send(Ok(ToolStreamEnd {
            stream_id: stream_id.clone(),
            title: result.title,
            summary: result.summary,
            output_text: result.output_text,
            sections: result.sections,
            payload: result.payload,
            metadata: result.metadata,
            attachments: result.attachments,
        }));
        Ok(ToolInvokeStream {
            stream_id,
            chunks: rx,
            end: end_rx,
        })
    }

    pub async fn dispatch_shell_env(
        &self,
        input: ShellEnvInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<ShellEnvPatch, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let plugins = self.plugins.clone();
        let mut set = std::collections::BTreeMap::new();
        let mut unset = Vec::new();
        for plugin in &plugins {
            if !plugin.subscribes(HookSubscription::SHELL_ENV) {
                continue;
            }
            let params = serde_json::to_value(&input)?;
            let result = await_transport_with_cancellation(
                cancellation.clone(),
                call_with_timeout(plugin, method::HOOK_SHELL_ENV, params, timeout),
            )
            .await
            .map_err(transport_to_plugin_error)?;
            if matches!(&result, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<ShellEnvPatch> = serde_json::from_value(result)
                .map_err(|error| PluginError::invalid_params(error.to_string()))?;
            if let Some(p) = patch {
                for (k, v) in p.set {
                    set.insert(k, v);
                }
                for k in p.unset {
                    set.remove(&k);
                    unset.push(k);
                }
            }
        }
        Ok(ShellEnvPatch { set, unset })
    }

    // -------------- async-only helpers for chat / permission etc. --------------

    pub async fn dispatch_chat_message(
        &self,
        input: ChatMessageInput,
    ) -> Result<ChatMessageInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        let session_id = Some(input.session_id);
        let mut runs = Vec::new();
        let result = dispatcher::chain_patch::<ChatMessageInput, ChatMessagePatch, _>(
            &self.plugins,
            method::HOOK_CHAT_MESSAGE,
            HookSubscription::CHAT_MESSAGE,
            timeout,
            input,
            |inp, patch| {
                if let Some(m) = patch.message {
                    inp.message = m;
                }
                if patch.drop {
                    inp.message.content = serde_json::Value::Null;
                }
            },
            session_id,
            &mut runs,
        )
        .await;
        // chat.message activity is intentionally not recorded (not part of
        // the transcript hook-run scope); the runs are discarded.
        let _ = runs;
        result.map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_chat_params(
        &self,
        input: ChatParamsInput,
    ) -> Result<ChatParamsInput, PluginError> {
        self.dispatch_chat_params_cancellable(input, None).await
    }

    pub async fn dispatch_chat_params_cancellable(
        &self,
        input: ChatParamsInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<ChatParamsInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        let session_id = input.session_id;
        let mut runs = Vec::new();
        let result = await_transport_with_cancellation(
            cancellation,
            dispatcher::chain_patch::<ChatParamsInput, ChatParamsPatch, _>(
                &self.plugins,
                method::HOOK_CHAT_PARAMS,
                HookSubscription::CHAT_PARAMS,
                timeout,
                input,
                |inp, patch| {
                    if let Some(p) = patch.params {
                        merge_json(&mut inp.params, p);
                    }
                },
                session_id,
                &mut runs,
            ),
        )
        .await;
        self.push_hook_runs(runs);
        result.map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_chat_headers(
        &self,
        input: ChatHeadersInput,
    ) -> Result<ChatHeadersInput, PluginError> {
        self.dispatch_chat_headers_cancellable(input, None).await
    }

    pub async fn dispatch_chat_headers_cancellable(
        &self,
        input: ChatHeadersInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<ChatHeadersInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        let mut runs = Vec::new();
        let result = await_transport_with_cancellation(
            cancellation,
            dispatcher::chain_patch::<ChatHeadersInput, ChatHeadersPatch, _>(
                &self.plugins,
                method::HOOK_CHAT_HEADERS,
                HookSubscription::CHAT_HEADERS,
                timeout,
                input,
                |inp, patch| {
                    for (k, v) in patch.set {
                        inp.headers.insert(k, v);
                    }
                    for k in patch.remove {
                        inp.headers.remove(&k);
                    }
                },
                None,
                &mut runs,
            ),
        )
        .await;
        // chat.headers activity is intentionally not recorded (not part of
        // the transcript hook-run scope); the runs are discarded.
        let _ = runs;
        result.map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_chat_system_transform(
        &self,
        input: ChatSystemTransformInput,
    ) -> Result<ChatSystemTransformInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        let session_id = Some(input.session_id);
        let mut runs = Vec::new();
        let result = dispatcher::chain_patch_in_context::<
            ChatSystemTransformInput,
            ChatSystemTransformPatch,
            _,
            _,
        >(
            &self.plugins,
            method::HOOK_CHAT_SYSTEM_TRANSFORM,
            HookSubscription::CHAT_SYSTEM_TRANSFORM,
            timeout,
            input,
            |inp, patch| {
                if let Some(p) = patch.prepend {
                    inp.current_system = format!("{p}\n{}", inp.current_system);
                }
                if let Some(a) = patch.append {
                    inp.current_system = format!("{}\n{a}", inp.current_system);
                }
                if let Some(s) = patch.system {
                    inp.current_system = s;
                }
            },
            |plugin, input| {
                Some(HostCallbackContext {
                    plugin_id: Some(plugin.key().to_string()),
                    session_id: Some(input.session_id),
                    ..Default::default()
                })
            },
            session_id,
            &mut runs,
        )
        .await;
        // chat.system.transform activity is intentionally not recorded (not
        // part of the transcript hook-run scope); the runs are discarded.
        let _ = runs;
        result.map_err(transport_to_plugin_error)
    }

    pub async fn broadcast_notification(&self, input: NotificationInput) {
        let timeout = Duration::from_secs(5);
        let mut notifications = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::NOTIFICATION) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            notifications.push(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_NOTIFICATION, params),
                )
                .await;
            });
        }
        futures_util::future::join_all(notifications).await;
    }

    pub async fn dispatch_command_before(
        &self,
        input: CommandBeforeInput,
    ) -> Result<CommandBeforeOutcome, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let session_id = input.session_id;
        let mut current = input;
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::COMMAND_BEFORE) {
                continue;
            }
            let plugin_id = plugin.key().to_string();
            let params = serde_json::to_value(&current)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = match call_with_timeout(plugin, method::HOOK_COMMAND_BEFORE, params, timeout)
                .await
            {
                Ok(v) => v,
                Err(err) => {
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        "command.before",
                        &plugin_id,
                        session_id,
                        &err,
                    )]);
                    return Err(transport_to_plugin_error(err));
                }
            };
            if matches!(&v, serde_json::Value::Null) {
                // No-op run; not recorded as transcript activity.
                continue;
            }
            let resp: Option<CommandBeforeResponse> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            match resp {
                Some(CommandBeforeResponse::Abort { reason }) => {
                    self.push_hook_runs(vec![HookRunRecord::new(
                        "command.before",
                        &plugin_id,
                        session_id,
                        HookRunStatus::Applied,
                        format!("command.before hook aborted command: {reason}"),
                        Some(reason.clone()),
                    )]);
                    return Ok(CommandBeforeOutcome::Abort(reason));
                }
                Some(CommandBeforeResponse::Patch(p)) => {
                    self.push_hook_runs(vec![HookRunRecord::new(
                        "command.before",
                        &plugin_id,
                        session_id,
                        HookRunStatus::Applied,
                        "command.before hook ran",
                        None,
                    )]);
                    if let Some(c) = p.command {
                        current.command = c;
                    }
                    if let Some(a) = p.args {
                        current.args = a;
                    }
                    if let Some(c) = p.cwd {
                        current.cwd = c;
                    }
                    if let Some(env) = p.env {
                        for (k, v) in env {
                            current.env.insert(k, v);
                        }
                    }
                }
                None => {
                    // No-op run; not recorded as transcript activity.
                }
            }
        }
        Ok(CommandBeforeOutcome::Continue(current))
    }

    pub async fn dispatch_auth(&self, input: AuthInput) -> Result<Option<AuthOutput>, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::AUTH) {
                continue;
            }
            let plugin_id = plugin.key().to_string();
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = match call_with_timeout(plugin, method::HOOK_AUTH, params, timeout).await {
                Ok(v) => v,
                Err(err) => {
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        "auth", &plugin_id, None, &err,
                    )]);
                    return Err(transport_to_plugin_error(err));
                }
            };
            if matches!(&v, serde_json::Value::Null) {
                // No-op run (no credentials supplied); not recorded.
                continue;
            }
            let out: Option<AuthOutput> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if out.is_some() {
                self.push_hook_runs(vec![HookRunRecord::new(
                    "auth",
                    &plugin_id,
                    None,
                    HookRunStatus::Applied,
                    "auth hook provided credentials",
                    None,
                )]);
                return Ok(out);
            }
        }
        Ok(None)
    }

    pub async fn dispatch_provider_list(
        &self,
        input: ProviderListInput,
    ) -> Result<ProviderListPatch, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let mut add = Vec::new();
        let mut remove = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::PROVIDER_LIST) {
                continue;
            }
            let plugin_id = plugin.key().to_string();
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = match call_with_timeout(plugin, method::HOOK_PROVIDER_LIST, params, timeout)
                .await
            {
                Ok(v) => v,
                Err(err) => {
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        "provider.list",
                        &plugin_id,
                        None,
                        &err,
                    )]);
                    return Err(transport_to_plugin_error(err));
                }
            };
            if matches!(&v, serde_json::Value::Null) {
                // No-op run; not recorded as transcript activity.
                continue;
            }
            let patch: Option<ProviderListPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                self.push_hook_runs(vec![HookRunRecord::new(
                    "provider.list",
                    &plugin_id,
                    None,
                    HookRunStatus::Applied,
                    "provider.list hook ran",
                    None,
                )]);
                add.extend(p.add);
                remove.extend(p.remove);
            }
        }
        Ok(ProviderListPatch { add, remove })
    }

    pub async fn dispatch_config(&self, input: ConfigInput) -> Result<ConfigInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let mut runs = Vec::new();
        let result = dispatcher::chain_patch::<ConfigInput, ConfigPatch, _>(
            &self.plugins,
            method::HOOK_CONFIG,
            HookSubscription::CONFIG,
            timeout,
            input,
            |inp, patch| {
                if let Some(m) = patch.merge {
                    merge_json(&mut inp.current, m);
                }
            },
            None,
            &mut runs,
        )
        .await;
        self.push_hook_runs(runs);
        result.map_err(transport_to_plugin_error)
    }

    // ── run lifecycle ──────────────────────────────────────────────────────

    pub async fn broadcast_pre_run(&self, input: PreRunInput) {
        self.broadcast_lifecycle(method::HOOK_PRE_RUN, HookSubscription::PRE_RUN, input)
            .await;
    }

    pub async fn broadcast_post_run(&self, input: PostRunInput) {
        self.broadcast_lifecycle(method::HOOK_POST_RUN, HookSubscription::POST_RUN, input)
            .await;
    }

    pub(super) async fn broadcast_lifecycle<T>(
        &self,
        method: &'static str,
        subscription: HookSubscription,
        input: T,
    ) where
        T: serde::Serialize + Clone + Send + 'static,
    {
        let timeout = Duration::from_secs(5);
        let mut notifications = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(subscription) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            notifications.push(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ =
                    tokio::time::timeout(timeout, plugin.transport.notify(method, params)).await;
            });
        }
        futures_util::future::join_all(notifications).await;
    }

    // ── session.start ──────────────────────────────────────────────────────

    pub async fn dispatch_session_start(
        &self,
        input: SessionStartInput,
    ) -> Result<SessionStartPatch, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        let session_id = Some(input.session_id);
        let mut additional_context: Option<String> = None;
        let mut initial_user_message = None;
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::SESSION_START) {
                continue;
            }
            let plugin_id = plugin.key().to_string();
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = match call_with_timeout(plugin, method::HOOK_SESSION_START, params, timeout)
                .await
            {
                Ok(v) => v,
                Err(err) => {
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        "session.start",
                        &plugin_id,
                        session_id,
                        &err,
                    )]);
                    return Err(transport_to_plugin_error(err));
                }
            };
            if matches!(&v, serde_json::Value::Null) {
                // No-op run; only effective runs (and failures) are recorded
                // as transcript activity.
                continue;
            }
            let patch: Option<SessionStartPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                self.push_hook_runs(vec![HookRunRecord::new(
                    "session.start",
                    &plugin_id,
                    session_id,
                    HookRunStatus::Applied,
                    "session.start hook ran",
                    None,
                )]);
                if let Some(ctx) = p.additional_context {
                    let existing = additional_context.get_or_insert_with(String::new);
                    if !existing.is_empty() {
                        existing.push('\n');
                    }
                    existing.push_str(&ctx);
                }
                if p.initial_user_message.is_some() {
                    initial_user_message = p.initial_user_message;
                }
            }
        }
        Ok(SessionStartPatch {
            additional_context,
            initial_user_message,
        })
    }

    // ── session.end ────────────────────────────────────────────────────────

    pub async fn broadcast_session_end(&self, input: SessionEndInput) {
        let timeout = Duration::from_secs(5);
        let mut notifications = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::SESSION_END) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            notifications.push(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(error) => {
                        return HookRunRecord::new(
                            "session.end",
                            plugin.key().to_string(),
                            Some(input.session_id),
                            HookRunStatus::Failed,
                            "session.end hook input serialization failed",
                            Some(error.to_string()),
                        );
                    }
                };
                let context = HostCallbackContext {
                    plugin_id: Some(plugin.key().to_string()),
                    session_id: Some(input.session_id),
                    ..Default::default()
                };
                let notified = tokio::time::timeout(
                    timeout,
                    host_api::run_in_host_callback_context(
                        context,
                        plugin.transport.notify(method::HOOK_SESSION_END, params),
                    ),
                )
                .await;
                if notified.is_ok() {
                    HookRunRecord::new(
                        "session.end",
                        plugin.key().to_string(),
                        Some(input.session_id),
                        HookRunStatus::Applied,
                        "session.end hook notified",
                        None,
                    )
                } else {
                    HookRunRecord::new(
                        "session.end",
                        plugin.key().to_string(),
                        Some(input.session_id),
                        HookRunStatus::TimedOut,
                        "session.end hook timed out",
                        None,
                    )
                }
            });
        }
        let records = futures_util::future::join_all(notifications).await;
        self.push_hook_runs(records);
    }

    // ── user.prompt.submit ─────────────────────────────────────────────────

    pub async fn dispatch_user_prompt_submit(
        &self,
        input: UserPromptSubmitInput,
    ) -> Result<UserPromptSubmitInput, PluginError> {
        self.dispatch_user_prompt_submit_cancellable(input, None)
            .await
    }

    pub async fn dispatch_user_prompt_submit_cancellable(
        &self,
        input: UserPromptSubmitInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<UserPromptSubmitInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        let session_id = Some(input.session_id);
        let mut current = input;
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::USER_PROMPT_SUBMIT) {
                continue;
            }
            let plugin_id = plugin.key().to_string();
            let params = serde_json::to_value(&current)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = match await_transport_with_cancellation(
                cancellation.clone(),
                call_with_timeout(plugin, method::HOOK_USER_PROMPT_SUBMIT, params, timeout),
            )
            .await
            {
                Ok(v) => v,
                Err(err) => {
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        "user.prompt.submit",
                        &plugin_id,
                        session_id,
                        &err,
                    )]);
                    return Err(transport_to_plugin_error(err));
                }
            };
            if matches!(&v, serde_json::Value::Null) {
                // No-op run; only effective runs (block/rewrite) and failures
                // are recorded as transcript activity.
                continue;
            }
            let patch: Option<UserPromptSubmitPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                if let Some(r) = p.block_reason {
                    self.push_hook_runs(vec![HookRunRecord::new(
                        "user.prompt.submit",
                        &plugin_id,
                        session_id,
                        HookRunStatus::Applied,
                        format!("user.prompt.submit hook blocked prompt: {r}"),
                        Some(r.clone()),
                    )]);
                    return Err(PluginError::internal(format!("prompt blocked: {r}")));
                }
                self.push_hook_runs(vec![HookRunRecord::new(
                    "user.prompt.submit",
                    &plugin_id,
                    session_id,
                    HookRunStatus::Applied,
                    "user.prompt.submit hook ran",
                    None,
                )]);
                if let Some(text) = p.prompt {
                    current.prompt = text;
                }
                if let Some(ctx) = p.additional_context {
                    current.prompt.push('\n');
                    current.prompt.push_str(&ctx);
                }
            }
        }
        Ok(current)
    }

    // ── tool.execute.failure ───────────────────────────────────────────────

    pub async fn broadcast_tool_failure(&self, input: ToolFailureInput) {
        let timeout = Duration::from_secs(5);
        let mut notifications = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::TOOL_FAILURE) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            notifications.push(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_TOOL_FAILURE, params),
                )
                .await;
            });
        }
        futures_util::future::join_all(notifications).await;
    }

    // ── tool.definition ────────────────────────────────────────────────────

    pub async fn dispatch_tool_definition(
        &self,
        input: ToolDefinitionInput,
    ) -> Result<ToolDefinitionInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let mut runs = Vec::new();
        let result =
            dispatcher::chain_patch_in_context::<ToolDefinitionInput, ToolDefinitionPatch, _, _>(
                &self.plugins,
                method::HOOK_TOOL_DEFINITION,
                HookSubscription::TOOL_DEFINITION,
                timeout,
                input,
                |inp, patch| {
                    if let Some(summary) = patch.summary {
                        inp.summary = summary;
                    }
                    if patch.help.is_some() {
                        inp.help = patch.help;
                    }
                    if let Some(s) = patch.input_schema {
                        inp.input_schema = s;
                    }
                },
                |plugin, input| {
                    Some(tool_hook_context(
                        plugin,
                        input.tool_name(),
                        None,
                        None,
                        None,
                    ))
                },
                None,
                &mut runs,
            )
            .await;
        // tool.definition runs once per registered tool per tool-request
        // computation; recording them as transcript activity would flood the
        // session with thousands of "no change" parts every turn. The chain
        // still applies its patches — only the runs are discarded, like
        // tool.after.
        let _ = runs;
        result.map_err(transport_to_plugin_error)
    }

    /// Apply `tool.definition` hooks to one registry snapshot while crossing
    /// the async boundary only once. Catalog consumers commonly need every
    /// definition together; entering a blocking runtime once per tool causes
    /// unnecessary scheduler churn and can exhaust the blocking pool when
    /// several catalog readers run concurrently.
    pub async fn dispatch_tool_definitions(
        &self,
        inputs: Vec<ToolDefinitionInput>,
    ) -> Vec<Result<ToolDefinitionInput, PluginError>> {
        const CATALOG_DEADLINE: Duration = Duration::from_secs(5);
        let input_count = inputs.len();
        let dispatch = async {
            let mut outputs = Vec::with_capacity(input_count);
            for input in inputs {
                outputs.push(self.dispatch_tool_definition(input).await);
            }
            outputs
        };
        match tokio::time::timeout(CATALOG_DEADLINE, dispatch).await {
            Ok(outputs) => outputs,
            Err(_) => (0..input_count)
                .map(|_| {
                    Err(PluginError::internal(
                        "tool.definition catalog pass did not complete within 5s",
                    ))
                })
                .collect(),
        }
    }

    // ── agent.stop ─────────────────────────────────────────────────────────

    pub async fn dispatch_agent_stop(
        &self,
        input: AgentStopInput,
    ) -> Result<AgentStopPatch, PluginError> {
        Ok(self
            .dispatch_agent_stop_cancellable(input, None)
            .await?
            .patch)
    }

    pub async fn dispatch_agent_stop_cancellable(
        &self,
        input: AgentStopInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<AgentStopDispatch, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(30));
        let mut continue_with_message = None;
        let mut reason = None;
        let mut runs = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::AGENT_STOP) {
                continue;
            }
            let plugin_id = plugin.key().to_string();
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let context = HostCallbackContext {
                plugin_id: Some(plugin_id.clone()),
                session_id: Some(input.session_id),
                ..Default::default()
            };
            let v = match await_transport_with_cancellation(
                cancellation.clone(),
                host_api::run_in_host_callback_context(
                    context,
                    call_with_timeout(plugin, method::HOOK_AGENT_STOP, params, timeout),
                ),
            )
            .await
            {
                Ok(v) => v,
                Err(err) => {
                    // One plugin's hook failure must not hide the other hooks'
                    // decisions (for example the workflow plan autorun
                    // continuation) nor the run's hook activity. Record the
                    // failure as an observed run and keep dispatching. A
                    // cancelled run still aborts immediately.
                    if cancellation
                        .as_ref()
                        .is_some_and(|token| token.is_cancelled())
                    {
                        return Err(transport_to_plugin_error(err));
                    }
                    tracing::warn!(
                        target: "agena_plugin_host::agent_stop",
                        plugin = %plugin_id,
                        error = %err,
                        "agent.stop hook failed; continuing to remaining plugins"
                    );
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        "agent.stop",
                        &plugin_id,
                        Some(input.session_id),
                        &err,
                    )]);
                    runs.push(AgentStopHookRun {
                        plugin_id,
                        hook: "agent.stop".to_string(),
                        continue_with_message: None,
                        reason: Some(format!("hook failed: {err}")),
                    });
                    continue;
                }
            };
            if matches!(&v, serde_json::Value::Null) {
                // No continuation: the run is a no-op and is not recorded as
                // transcript activity. The dispatch still notes it for the
                // AgentStop aggregation.
                runs.push(AgentStopHookRun::ran(plugin_id, "agent.stop"));
                continue;
            }
            let patch: Option<AgentStopPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                let has_effect = p.continue_with_message.is_some() || p.reason.is_some();
                if has_effect {
                    let (summary, detail) = match (&p.continue_with_message, &p.reason) {
                        (Some(_), Some(reason)) => (
                            format!("agent.stop hook blocked stop: {reason}"),
                            p.continue_with_message.clone().or_else(|| p.reason.clone()),
                        ),
                        (Some(_), None) => (
                            "agent.stop hook blocked stop".to_string(),
                            p.continue_with_message.clone(),
                        ),
                        (None, Some(reason)) => (
                            format!("agent.stop hook ran: {reason}"),
                            Some(reason.clone()),
                        ),
                        (None, None) => unreachable!("covered by has_effect"),
                    };
                    self.push_hook_runs(vec![HookRunRecord::new(
                        "agent.stop",
                        &plugin_id,
                        Some(input.session_id),
                        HookRunStatus::Applied,
                        summary,
                        detail,
                    )]);
                }
                runs.push(AgentStopHookRun {
                    plugin_id,
                    hook: "agent.stop".to_string(),
                    continue_with_message: p.continue_with_message.clone(),
                    reason: p.reason.clone(),
                });
                if p.continue_with_message.is_some() {
                    continue_with_message = p.continue_with_message;
                    reason = p.reason;
                    // First plugin that wants to block stop wins.
                    break;
                }
            } else {
                // A deserialized `None` patch is a no-op (no continuation);
                // not recorded as transcript activity.
                runs.push(AgentStopHookRun::ran(plugin_id, "agent.stop"));
            }
        }
        Ok(AgentStopDispatch {
            patch: AgentStopPatch {
                continue_with_message,
                reason,
            },
            runs,
        })
    }

    // ── command.execute.after ──────────────────────────────────────────────

    pub async fn dispatch_command_after(
        &self,
        input: CommandAfterInput,
    ) -> Result<CommandAfterInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        let session_id = input.session_id;
        let mut runs = Vec::new();
        let result = dispatcher::chain_patch::<CommandAfterInput, CommandAfterPatch, _>(
            &self.plugins,
            method::HOOK_COMMAND_AFTER,
            HookSubscription::COMMAND_AFTER,
            timeout,
            input,
            |inp, patch| {
                if let Some(s) = patch.stdout {
                    inp.stdout = s;
                }
                if let Some(s) = patch.stderr {
                    inp.stderr = s;
                }
                if patch.exit_code.is_some() {
                    inp.exit_code = patch.exit_code;
                }
            },
            session_id,
            &mut runs,
        )
        .await;
        self.push_hook_runs(runs);
        result.map_err(transport_to_plugin_error)
    }

    // ── chat.messages.transform ────────────────────────────────────────────

    pub async fn dispatch_chat_messages_transform(
        &self,
        input: ChatMessagesTransformInput,
    ) -> Result<ChatMessagesTransformInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(10));
        let session_id = Some(input.session_id);
        let mut runs = Vec::new();
        let result =
            dispatcher::chain_patch::<ChatMessagesTransformInput, ChatMessagesTransformPatch, _>(
                &self.plugins,
                method::HOOK_CHAT_MESSAGES_TRANSFORM,
                HookSubscription::CHAT_MESSAGES_TRANSFORM,
                timeout,
                input,
                |inp, patch| {
                    if let Some(msgs) = patch.messages {
                        inp.messages = msgs;
                    }
                },
                session_id,
                &mut runs,
            )
            .await;
        // chat.messages.transform activity is intentionally not recorded (not
        // part of the transcript hook-run scope); the runs are discarded.
        let _ = runs;
        result.map_err(transport_to_plugin_error)
    }

    /// Push an `EventEnvelope` to every subscribed plugin (best-effort, no
    /// error propagation — events are notifications).
    pub async fn broadcast_event(&self, env: EventEnvelope) {
        let timeout = Duration::from_secs(2);
        let mut notifications = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::EVENT) {
                continue;
            }
            let env = env.clone();
            let plugin = plugin.clone();
            notifications.push(async move {
                let params = match serde_json::to_value(&env) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_EVENT, params),
                )
                .await;
            });
        }
        futures_util::future::join_all(notifications).await;
    }

    /// Async shutdown — sends `meta/shutdown` and closes every transport.
    /// Plugins whose transport has been transferred to a successor host during
    /// hot-reload are skipped.
    pub async fn shutdown(&self) {
        let transferred = self.transferred_to_successor.lock().await.clone();
        for plugin in &self.plugins {
            if transferred.contains(&plugin.key()) {
                continue;
            }
            let _ = shutdown_transport(Arc::clone(&plugin.transport)).await;
        }
    }

    /// Direct access to the host-side bidirectional router. Used by HTTP
    /// callback routes and bidirectional stdio transports.
    pub fn host_handle(&self) -> Arc<HostHandle> {
        Arc::clone(&self._host_handle)
    }

    pub fn display_contributions(&self) -> Vec<HostDisplayContribution> {
        self.ui_catalog().tui.display
    }

    pub fn theme_palettes(&self) -> Vec<HostThemePalette> {
        self.ui_catalog().tui.themes
    }

    /// Plugin-emitted notifications through the unified `host.notify` entry
    /// (Phase 6). Bounded recent queue; frontends dedupe/consume by
    /// `plugin_id:severity:body`.
    pub fn host_notifications(&self) -> Vec<HostNotification> {
        self._host_handle.host_notifications()
    }

    pub fn studio_commands(&self) -> Vec<PluginCommandCatalogItem> {
        self.ui_catalog().studio.commands
    }

    pub fn studio_controls(&self) -> Vec<PluginStudioControlCatalogItem> {
        self.ui_catalog().studio.controls
    }

    pub fn studio_views(&self) -> Vec<PluginStudioViewCatalogItem> {
        self.ui_catalog().studio.views
    }

    pub fn ui_catalog(&self) -> PluginUiCatalog {
        let mut display_by_key = BTreeMap::<(PluginKey, String), HostDisplayContribution>::new();
        // Theme IDs are scoped by plugin in the manifest. Keep the owner in
        // the catalog key so two plugins cannot silently overwrite one
        // another while building the aggregate UI catalog.
        let mut themes_by_key = BTreeMap::<(PluginKey, String), HostThemePalette>::new();
        let mut studio_commands = Vec::new();
        let mut studio_controls = Vec::new();
        let mut studio_views = Vec::new();

        for plugin in &self.plugins {
            // Declarative manifest display contributions (Phase 6). Dynamic
            // runtime contributions arrive through the host-handle channel.
            for contribution in &plugin.manifest.ui.display {
                let resolved = HostDisplayContribution {
                    plugin_id: plugin.key(),
                    contribution: contribution.clone(),
                };
                display_by_key.insert(
                    (resolved.plugin_id.clone(), resolved.contribution.id.clone()),
                    resolved,
                );
            }

            for theme in &plugin.manifest.ui.tui.themes {
                themes_by_key.insert(
                    (plugin.key(), theme.id.clone()),
                    HostThemePalette {
                        id: theme.id.clone(),
                        plugin_id: plugin.key(),
                        display_name: theme.display_name.clone(),
                        colors: theme.colors.clone(),
                    },
                );
            }

            studio_commands.extend(plugin.manifest.commands.iter().cloned().map(|command| {
                PluginCommandCatalogItem {
                    plugin_id: plugin.key(),
                    command,
                }
            }));

            studio_controls.extend(plugin.manifest.ui.studio.controls.iter().cloned().map(
                |control| PluginStudioControlCatalogItem {
                    plugin_id: plugin.key(),
                    control,
                },
            ));

            studio_views.extend(plugin.manifest.ui.studio.views.iter().cloned().map(|view| {
                PluginStudioViewCatalogItem {
                    plugin_id: plugin.key(),
                    view,
                }
            }));
        }

        for contribution in self._host_handle.display_list_response() {
            display_by_key.insert(
                (
                    contribution.plugin_id.clone(),
                    contribution.contribution.id.clone(),
                ),
                contribution,
            );
        }
        for theme in self._host_handle.theme_list_response().themes {
            themes_by_key.insert((theme.plugin_id.clone(), theme.id.clone()), theme);
        }

        let mut display = display_by_key.into_values().collect::<Vec<_>>();
        display.sort_by(|a, b| {
            b.contribution
                .priority
                .cmp(&a.contribution.priority)
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
                .then_with(|| a.contribution.id.cmp(&b.contribution.id))
        });
        let themes = themes_by_key.into_values().collect::<Vec<_>>();
        studio_commands.sort_by(|a, b| {
            a.command
                .location
                .cmp(&b.command.location)
                .then_with(|| a.command.category.cmp(&b.command.category))
                .then_with(|| a.command.title.cmp(&b.command.title))
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
        });
        studio_controls.sort_by(|a, b| {
            a.control
                .location
                .cmp(&b.control.location)
                .then_with(|| a.control.title.cmp(&b.control.title))
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
        });
        studio_views.sort_by(|a, b| {
            a.view
                .location
                .cmp(&b.view.location)
                .then_with(|| a.view.title.cmp(&b.view.title))
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
        });

        PluginUiCatalog {
            tui: PluginTuiUiCatalog { display, themes },
            studio: PluginStudioUiCatalog {
                commands: studio_commands,
                controls: studio_controls,
                views: studio_views,
            },
        }
    }

    pub fn resolve_studio_action(
        &self,
        plugin_id: &str,
        action_id: &str,
    ) -> Option<PluginUiAction> {
        let plugin_key: PluginKey = plugin_id.parse().ok()?;
        let plugin = self.plugins_by_id.get(&plugin_key)?;
        for command in &plugin.manifest.commands {
            if command.id == action_id {
                return Some(command.action.clone());
            }
        }
        for control in &plugin.manifest.ui.studio.controls {
            if control.id == action_id {
                return Some(control.action.clone());
            }
        }
        for view in &plugin.manifest.ui.studio.views {
            for control in &view.controls {
                if control.id == action_id {
                    return Some(control.action.clone());
                }
            }
        }
        None
    }

    pub fn resolve_registered_tool_for_plugin_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
    ) -> Option<RegisteredTool> {
        let plugin_key: PluginKey = plugin_id.parse().ok()?;
        let registry = self.tool_registry.read().ok()?;
        registry
            .lookup_for_plugin(&plugin_key, tool_name)
            .cloned()
            .or_else(|| {
                let tool_key: ToolKey = tool_name.parse().ok()?;
                (tool_key.plugin() == &plugin_key)
                    .then(|| registry.lookup_tool_by_key(&tool_key).cloned())
                    .flatten()
            })
    }

    /// Queue observed hook runs for later transcript recording.
    pub fn push_hook_runs(&self, runs: Vec<HookRunRecord>) {
        push_hook_runs_into(&self.hook_runs, runs);
    }

    /// Take every queued hook run attributed to `session_id`, plus runs
    /// recorded without a session (which the current session operation
    /// claims). Runs belonging to other sessions stay queued for their own
    /// consumption.
    pub fn drain_hook_runs(&self, session_id: i64) -> Vec<HookRunRecord> {
        let mut pending = self
            .hook_runs
            .lock()
            .expect("hook run queue mutex poisoned");
        let mut taken = Vec::new();
        let mut remaining = Vec::new();
        for run in pending.drain(..) {
            match run.session_id {
                Some(sid) if sid == session_id => taken.push(run),
                Some(_) => remaining.push(run),
                None => taken.push(run),
            }
        }
        *pending = remaining;
        taken
    }
}

async fn await_transport_with_cancellation<T, F>(
    cancellation: Option<tokio_util::sync::CancellationToken>,
    future: F,
) -> Result<T, TransportError>
where
    F: std::future::Future<Output = Result<T, TransportError>>,
{
    match cancellation {
        Some(cancellation) => tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(TransportError::Cancelled),
            result = future => result,
        },
        None => future.await,
    }
}
use super::{
    AgentStopDispatch, AgentStopHookRun, AgentStopInput, AgentStopPatch, Arc, AuthInput,
    AuthOutput, BTreeMap, ChatHeadersInput, ChatHeadersPatch, ChatMessageInput, ChatMessagePatch,
    ChatMessagesTransformInput, ChatMessagesTransformPatch, ChatParamsInput, ChatParamsPatch,
    ChatSystemTransformInput, ChatSystemTransformPatch, CommandAfterInput, CommandAfterPatch,
    CommandBeforeInput, CommandBeforeOutcome, CommandBeforeResponse, ConfigInput, ConfigPatch,
    Duration, EventEnvelope, HashMap, HookRunRecord, HookRunStatus, HookSubscription,
    HostCallbackContext, HostDisplayContribution, HostHandle, HostNotification, HostThemePalette,
    LoadedPlugin, NoopHostClient, NotificationInput, PluginCommandCatalogItem,
    PluginCommandInvokeInput, PluginCommandOutput, PluginError, PluginHost, PluginInspect,
    PluginKey, PluginLogRecord, PluginLogStore, PluginStudioControlCatalogItem,
    PluginStudioUiCatalog, PluginStudioViewCatalogItem, PluginToolRegistry, PluginTuiUiCatalog,
    PluginUiAction, PluginUiCatalog, PostRunInput, PreRunInput, ProviderListInput,
    ProviderListPatch, RegisteredTool, RwLock, SessionEndInput, SessionStartInput,
    SessionStartPatch, ShellEnvInput, ShellEnvPatch, TimeoutsConfig, ToolAfterInput,
    ToolAfterPatch, ToolBeforeInput, ToolBeforePatch, ToolDefinitionInput, ToolDefinitionPatch,
    ToolFailureInput, ToolInvokeInput, ToolInvokeOutput, ToolInvokeStream, ToolKey,
    ToolPermissionNetworksInput, ToolPermissionPathsInput, ToolRegistryChangedEvent,
    ToolStreamChunk, ToolStreamEnd, TransportError, UserPromptSubmitInput, UserPromptSubmitPatch,
    call_with_timeout, dispatcher, hook_registration_for_plugin, host_api, merge_json, method,
    push_hook_runs_into, shutdown_transport, tool_hook_context, transport_to_plugin_error,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginManifest;

    #[test]
    fn hook_run_queue_drains_by_session_and_claims_unattributed() {
        let host = PluginHost::new_empty();
        host.push_hook_runs(vec![
            HookRunRecord::new(
                "session.start",
                "test.p1",
                Some(1),
                HookRunStatus::Applied,
                "session.start hook ran",
                None,
            ),
            HookRunRecord::new(
                "config",
                "test.p2",
                None,
                HookRunStatus::Applied,
                "config hook ran",
                None,
            ),
            HookRunRecord::new(
                "chat.params",
                "test.p1",
                Some(2),
                HookRunStatus::Skipped,
                "chat.params hook ran (no change)",
                None,
            ),
        ]);

        let s1 = host.drain_hook_runs(1);
        assert_eq!(
            s1.len(),
            2,
            "session 1 claims its own run plus the unattributed one"
        );
        assert!(
            s1.iter()
                .all(|r| r.session_id.is_none() || r.session_id == Some(1))
        );

        let s2 = host.drain_hook_runs(2);
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].session_id, Some(2));

        assert!(host.drain_hook_runs(1).is_empty());
        assert!(host.drain_hook_runs(2).is_empty());
    }

    #[test]
    fn hook_run_queue_is_bounded_and_drops_oldest() {
        let host = PluginHost::new_empty();
        let total = super::super::MAX_PENDING_HOOK_RUNS + 10;
        let runs = (0..total)
            .map(|i| {
                HookRunRecord::new(
                    "config",
                    "test.p",
                    None,
                    HookRunStatus::Applied,
                    format!("run {i}"),
                    None,
                )
            })
            .collect::<Vec<_>>();
        host.push_hook_runs(runs);
        let drained = host.drain_hook_runs(1);
        assert_eq!(drained.len(), super::super::MAX_PENDING_HOOK_RUNS);
        assert_eq!(
            drained[0].summary,
            format!("run {}", total - super::super::MAX_PENDING_HOOK_RUNS)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wedged_tool_definition_hook_times_out_without_blocking_the_runtime() {
        use crate::transport::PluginTransport;

        struct HangingTransport;
        #[async_trait::async_trait]
        impl PluginTransport for HangingTransport {
            async fn dispatch(
                &self,
                _method: &str,
                _params: serde_json::Value,
            ) -> Result<serde_json::Value, TransportError> {
                std::future::pending().await
            }
        }

        let tool_registry = Arc::new(RwLock::new(PluginToolRegistry::new()));
        let statuses = Arc::new(crate::status::StatusRegistry::new());
        let logs = Arc::new(PluginLogStore::default());
        let host = Arc::new(PluginHost {
            plugins: vec![Arc::new(LoadedPlugin::new(
                "static",
                crate::config::ConfiguredPlugin {
                    enabled: true,
                    package: crate::config::PluginPackage::Static {},
                    config: serde_json::json!({}),
                    timeouts: TimeoutsConfig::default(),
                },
                Arc::new(HangingTransport),
                PluginManifest {
                    schema_version: 1,
                    namespace: "test".to_string(),
                    name: "hanging".to_string(),
                    version: "0.1.0".to_string(),
                    summary: None,
                    help: None,
                    authors: Vec::new(),
                    transports: Vec::new(),
                    hooks: HookSubscription::TOOL_DEFINITION,
                    tools: Vec::new(),
                    commands: Vec::new(),
                    activity_kinds: Vec::new(),
                    tags: Vec::new(),
                    skills: Vec::new(),
                    ui: Default::default(),
                    config_schema: None,
                    config_schema_i18n: Default::default(),
                },
                "test".to_string(),
                Vec::new(),
            ))],
            plugins_by_id: Default::default(),
            tool_registry: Arc::clone(&tool_registry),
            statuses: Arc::clone(&statuses),
            logs: Arc::clone(&logs),
            timeouts: TimeoutsConfig {
                fast: Some(crate::config::DurationSpec(Duration::from_millis(20))),
                ..Default::default()
            },
            _host_handle: Arc::new(HostHandle::new_with_components(
                Arc::new(NoopHostClient),
                Arc::clone(&tool_registry),
                Arc::new(RwLock::new(HashMap::new())),
                Arc::new(RwLock::new(HashMap::new())),
                Arc::clone(&statuses),
                Arc::clone(&logs),
                None,
            )),
            transferred_to_successor: tokio::sync::Mutex::new(Default::default()),
            hook_runs: Arc::new(std::sync::Mutex::new(Vec::new())),
        });

        let started = std::time::Instant::now();
        let result = host
            .dispatch_tool_definitions(vec![ToolDefinitionInput {
                tool: ToolKey::new(
                    PluginKey::new("test", "hanging").expect("valid key"),
                    "example",
                )
                .expect("valid tool key"),
                summary: "s".to_string(),
                help: None,
                input_schema: serde_json::json!({}),
            }])
            .await
            .pop()
            .expect("one result");
        assert!(result.is_err(), "wedged hook must time out: {result:?}");
        assert!(started.elapsed() < Duration::from_secs(1));

        tokio::time::timeout(Duration::from_millis(50), tokio::task::yield_now())
            .await
            .expect("current-thread runtime remains responsive");
    }
}
