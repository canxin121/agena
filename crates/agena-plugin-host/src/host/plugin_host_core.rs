fn parse_plugin_key_for_lookup(plugin_id: &str, command: &str) -> Option<PluginKey> {
    match plugin_id.parse() {
        Ok(key) => Some(key),
        Err(error) => {
            tracing::warn!(
                command,
                plugin_id,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "parse plugin identifier",
                    &error,
                ),
                "plugin lookup rejected an invalid identifier"
            );
            None
        }
    }
}

fn compact_tool_call_name(canonical_name: &str) -> String {
    let mut parts = canonical_name.splitn(3, '.');
    let _namespace = parts.next();
    let plugin = parts.next();
    let tool = parts.next();
    match (plugin, tool) {
        (Some(plugin), Some(tool)) if plugin == tool => plugin.to_string(),
        (Some(plugin), Some(tool)) => format!("{plugin}.{tool}"),
        _ => canonical_name.to_string(),
    }
}

fn registered_tool_matches_name(tool: &RegisteredTool, name: &str) -> bool {
    let canonical_name = tool.canonical_name();
    canonical_name == name
        || tool.claude_mod_tool_name().map_or_else(
            || compact_tool_call_name(canonical_name.as_str()) == name,
            |alias| alias == name,
        )
}

fn extend_hook_text(target: &mut Vec<String>, values: Vec<String>, available: &mut usize) {
    for mut value in values.into_iter().filter(|value| !value.trim().is_empty()) {
        if *available == 0 {
            break;
        }
        value.truncate(value.floor_char_boundary(value.len().min(*available)));
        *available = available.saturating_sub(value.len() + 1);
        if !value.is_empty() {
            target.push(value);
        }
    }
}

fn plugin_tool_registry_read<'a>(
    registry: &'a RwLock<PluginToolRegistry>,
    command: &str,
) -> std::sync::RwLockReadGuard<'a, PluginToolRegistry> {
    match registry.read() {
        Ok(registry) => registry,
        Err(error) => {
            tracing::error!(
                command,
                diagnostic = %error,
                "plugin tool-registry lock was poisoned; recovering its inner state"
            );
            error.into_inner()
        }
    }
}

struct PromptExpansionHooks {
    package_name: String,
    hooks: Vec<Arc<LoadedPlugin>>,
    runs: Arc<std::sync::Mutex<Vec<HookRunRecord>>>,
    timeout: Duration,
}

impl PromptExpansionHooks {
    async fn apply(&self, host: &HostHandle, input: &mut CommandInvokeInput) -> Result<(), String> {
        use crate::sdk::{UserPromptExpansionInput, UserPromptExpansionPatch};

        let Some(session_id) = input.session_id else {
            return Ok(());
        };
        let command_name = format!("{}:{}", self.package_name, input.command_id);
        let args = input
            .input
            .get("args")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&input.raw);
        let prompt = if args.is_empty() {
            format!("/{command_name}")
        } else {
            format!("/{command_name} {args}")
        };
        let params = serde_json::json!(UserPromptExpansionInput {
            session_id,
            command_name,
            command_args: args.to_owned(),
            prompt,
        });
        let method = method::HOOK_USER_PROMPT_EXPANSION;
        let event = "prompt.expand";
        let mut runs = Vec::new();
        let mut block = None;
        for plugin in &self.hooks {
            let id = plugin.key();
            let response = host
                .run_in_authorized_callback_context(
                    &id,
                    HostCallbackContext {
                        session_id: Some(session_id),
                        ..Default::default()
                    },
                    call_with_timeout(plugin, method, params.clone(), self.timeout),
                )
                .await;
            let value = match response {
                Ok(value) => value,
                Err(error) => {
                    runs.push(dispatcher::transport_failure_record(
                        event,
                        &id.to_string(),
                        Some(session_id),
                        &error,
                    ));
                    continue;
                }
            };
            let patch = match serde_json::from_value::<Option<UserPromptExpansionPatch>>(value) {
                Ok(Some(patch)) => patch,
                Ok(None) => continue,
                Err(error) => {
                    runs.push(HookRunRecord::new(
                        event,
                        &id.to_string(),
                        Some(session_id),
                        HookRunStatus::Failed,
                        "invalid prompt expansion patch",
                        Some(error.to_string()),
                    ));
                    continue;
                }
            };
            if let Some(reason) = patch
                .block_reason
                .filter(|reason| !reason.trim().is_empty())
            {
                let reason = reason.chars().take(8192).collect::<String>();
                runs.push(HookRunRecord::new(
                    event,
                    &id.to_string(),
                    Some(session_id),
                    HookRunStatus::Applied,
                    "prompt expansion blocked by hook",
                    Some(reason.clone()),
                ));
                block = Some(reason);
                break;
            }
            if let Some(context) = patch
                .additional_context
                .filter(|text| !text.trim().is_empty())
            {
                const MAX_CONTEXT_BYTES: usize = 16 * 1024;
                let available = MAX_CONTEXT_BYTES.saturating_sub(input.prompt_context.len());
                if available > 1 {
                    if !input.prompt_context.is_empty() {
                        input.prompt_context.push('\n');
                    }
                    let end = context.floor_char_boundary(context.len().min(available - 1));
                    input.prompt_context.push_str(&context[..end]);
                }
                runs.push(HookRunRecord::new(
                    event,
                    &id.to_string(),
                    Some(session_id),
                    HookRunStatus::Applied,
                    "prompt expansion hook added context",
                    None,
                ));
            }
        }
        push_hook_runs_into(&self.runs, runs);
        block.map_or(Ok(()), Err)
    }
}

impl PluginHost {
    /// Activate business-dependent plugin work only after Application is bound.
    pub async fn application_ready(&self) -> Result<(), PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(30));
        for plugin in &self.plugins {
            if !plugin
                .manifest
                .hooks
                .contains(HookSubscription::APPLICATION_READY)
            {
                continue;
            }
            self.call_hook_with_default_context(
                plugin,
                method::META_READY,
                serde_json::json!({}),
                timeout,
            )
            .await
            .map_err(transport_to_plugin_error)?;
        }
        Ok(())
    }

    // Even hooks without session/tool privileges need an originating call
    // authority, so control callbacks can defer retirement until they return.
    async fn call_hook_with_default_context(
        &self,
        plugin: &LoadedPlugin,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, TransportError> {
        self._host_handle
            .run_in_authorized_callback_context(
                &plugin.key(),
                HostCallbackContext::default(),
                call_with_timeout(plugin, method, params, timeout),
            )
            .await
    }

    /// Dispatch one request/response hook with the owning session only.
    ///
    /// A hook that belongs to a session may delegate work to another plugin's
    /// tool (`HostClient::invoke_tool`). It never carries a call id, so the
    /// delegation is authorized under the session's policy rather than a
    /// transcript tool slot it does not have.
    ///
    /// Only `dispatch`-style hooks use this. A one-way `notify` returns when
    /// the frame is written, so its callback authority would already be
    /// revoked before the handler ran.
    pub(super) async fn call_session_hook(
        &self,
        plugin: &LoadedPlugin,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
        session_id: Option<i64>,
    ) -> Result<serde_json::Value, TransportError> {
        self._host_handle
            .run_in_authorized_callback_context(
                &plugin.key(),
                HostCallbackContext {
                    session_id,
                    ..HostCallbackContext::default()
                },
                call_with_timeout(plugin, method, params, timeout),
            )
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn chain_patch_with_default_context<I, P, F>(
        &self,
        method: &str,
        subscription: HookSubscription,
        timeout: Duration,
        input: I,
        apply: F,
        session_id: Option<i64>,
        runs: &mut Vec<HookRunRecord>,
    ) -> Result<I, TransportError>
    where
        I: serde::Serialize + Clone,
        P: serde::de::DeserializeOwned,
        F: FnMut(&mut I, P),
    {
        dispatcher::chain_patch_in_context(
            &self.plugins,
            method,
            subscription,
            timeout,
            input,
            apply,
            move |_, _| {
                Some(HostCallbackContext {
                    session_id,
                    ..HostCallbackContext::default()
                })
            },
            Some(&self._host_handle),
            session_id,
            runs,
        )
        .await
    }

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
        let command_registry = host_handle.command_registry();
        Arc::new(Self {
            sandbox_enabled: agena_process::sandbox::enabled(),
            plugins: Vec::new(),
            plugins_by_id: HashMap::new(),
            tool_registry,
            command_registry,
            command_pipeline: Arc::new(crate::event_pipeline::PluginAroundPipeline::new()),
            tool_before_pipeline: Arc::new(
                crate::event_pipeline::PluginTransformBailPipeline::new(
                    crate::event_pipeline::PluginPipelineFailurePolicy::Abort,
                ),
            ),
            tool_after_pipeline: Arc::new(crate::event_pipeline::PluginTransformPipeline::new(
                crate::event_pipeline::PluginPipelineFailurePolicy::Abort,
            )),
            lifecycle: super::lifecycle_hooks::LifecycleHooks::empty(),
            statuses,
            logs,
            configured_plugins: BTreeMap::new(),
            activation_blocks: BTreeMap::new(),
            activation_epochs: BTreeMap::new(),
            reload_plan: Default::default(),
            profile_resolution: Default::default(),
            prefetched_manifests: BTreeMap::new(),
            service_bindings: BTreeMap::new(),
            timeouts: TimeoutsConfig::default(),
            _host_handle: host_handle,
            transferred_to_successor: Arc::new(tokio::sync::Mutex::new(Default::default())),
            shutdown_result: Default::default(),
            hook_runs: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn has_subscription(&self, hook: HookSubscription) -> bool {
        self.plugins.iter().any(|plugin| plugin.subscribes(hook))
    }

    pub fn imported_subagent_type(&self, profile: &str) -> String {
        for plugin in &self.plugins {
            if let Some(package) = plugin.agent_plugin()
                && let Some(role) = profile.strip_prefix(&format!("{}.", plugin.key()))
                && package
                    .roles
                    .iter()
                    .any(|definition| definition.name == role && !definition.output_style)
            {
                return format!("{}:{role}", package.name);
            }
        }
        profile.to_owned()
    }

    pub fn plugins(&self) -> &[Arc<LoadedPlugin>] {
        &self.plugins
    }

    pub fn agent_plugin_skill_dirs(&self) -> Vec<std::path::PathBuf> {
        self.plugins
            .iter()
            .filter_map(|plugin| plugin.agent_plugin())
            .flat_map(|package| package.skill_dirs.iter().cloned())
            .collect()
    }

    pub async fn agent_plugin_provided_skills(
        &self,
        cwd: &std::path::Path,
        session: Option<i64>,
        is_enabled: impl Fn(&PluginKey) -> bool + Send,
    ) -> Vec<crate::AgentPluginSkill> {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut skills = Vec::new();
        for plugin in &self.plugins {
            if std::time::Instant::now() >= deadline {
                break;
            }
            let key = plugin.key();
            let Some(package) = plugin.agent_plugin().filter(|_| is_enabled(&key)) else {
                continue;
            };
            let effect = plugin.effect_scope();
            let Ok(_lease) = effect.lease() else {
                continue;
            };
            let stop = effect.cancellation_token();
            let id = key.to_string();
            let host = self
                ._host_handle
                .scoped_host_client_for_scope(id.clone(), effect);
            let observed = tokio::select! {
                biased;
                _ = stop.cancelled() => continue,
                observed = self._host_handle.run_in_authorized_callback_context(
                    &key,
                    HostCallbackContext {
                        session_id: session,
                        workspace_root: session.map(|_| cwd.to_string_lossy().into_owned()),
                        ..Default::default()
                    },
                    package.provided_skills(cwd, deadline, session, host.as_ref()),
                ) => observed,
            };
            skills.extend(
                observed.into_iter()
                    .map(|skill| skill.into_declared(&id, &plugin.manifest.version)),
            );
        }
        skills
    }

    pub fn agent_plugin_bin_dirs(&self) -> impl Iterator<Item = &std::path::Path> {
        self.plugins
            .iter()
            .filter_map(|plugin| plugin.agent_plugin()?.bin_dir.as_deref())
    }

    /// Host-only, explicitly pinned integration service call. It supplies no
    /// session/tool authority: an adapter cannot turn a polling lease into a
    /// filesystem/tool invocation in the owner's conversation. Plugin transport
    /// lifecycle, schema validation and bounded timeout remain unchanged.
    pub async fn invoke_integration_service(
        &self,
        provider: &str,
        method_name: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        let key: PluginKey = provider
            .parse()
            .map_err(|e| PluginError::invalid_params(format!("invalid provider: {e}")))?;
        let plugin = self
            .plugins_by_id
            .get(&key)
            .ok_or_else(|| PluginError::invalid_params("integration provider is not active"))?;
        let export = plugin
            .manifest
            .services
            .exports
            .iter()
            .find(|s| s.id == crate::sdk::integration::INTEGRATION_SERVICE && s.api_version == 1)
            .ok_or_else(|| {
                PluginError::invalid_params("provider does not export agena.integration API v1")
            })?;
        let contract = export
            .methods
            .iter()
            .find(|m| m.id == method_name)
            .ok_or_else(|| PluginError::invalid_params("integration method is not exported"))?;
        if !matches!(method_name, "describe" | "poll" | "send") {
            return Err(PluginError::invalid_params(
                "unsupported integration operation",
            ));
        }
        contract
            .input
            .validate_value(&input)
            .map_err(|e| PluginError::invalid_params(format!("invalid adapter input: {e}")))?;
        let request = crate::sdk::PluginServiceInvokeInput {
            service: crate::sdk::integration::INTEGRATION_SERVICE.into(),
            api_version: 1,
            method: method_name.into(),
            input,
        };
        let params =
            serde_json::to_value(request).map_err(|e| PluginError::invalid_params_error(&e))?;
        let value = self
            .call_hook_with_default_context(
                plugin,
                crate::sdk::rpc::method::SERVICE_INVOKE,
                params,
                Duration::from_secs(if method_name == "describe" { 3 } else { 20 }),
            )
            .await
            .map_err(|e| PluginError::internal(format!("adapter transport failed: {e}")))?;
        if serde_json::to_vec(&value)
            .map_err(|e| PluginError::internal_error(&e))?
            .len()
            > 1024 * 1024
        {
            return Err(PluginError::internal("adapter output exceeds 1 MiB"));
        }
        contract
            .output
            .validate_value(&value)
            .map_err(|e| PluginError::internal(format!("invalid adapter output: {e}")))?;
        Ok(value)
    }

    pub fn plugin_summary(&self) -> (usize, BTreeMap<&'static str, usize>) {
        let mut by_kind: BTreeMap<&'static str, usize> = BTreeMap::new();
        for p in &self.plugins {
            *by_kind.entry(p.kind).or_insert(0) += 1;
        }
        (self.plugins.len(), by_kind)
    }

    pub fn lookup_tool(&self, canonical_name: &str) -> Option<RegisteredTool> {
        let scope = host_api::current_host_callback_context()
            .and_then(|context| context.session_id)
            .map(PluginScopeKey::session);
        self.lookup_tool_for_scope(canonical_name, scope.as_ref())
    }

    /// Resolve a model-facing tool name. Canonical registry keys are checked
    /// through the strict lookup path; compact `plugin.tool` aliases are
    /// resolved only when exactly one visible tool owns the alias.
    pub fn lookup_tool_for_name(&self, name: &str) -> Option<RegisteredTool> {
        let scope = host_api::current_host_callback_context()
            .and_then(|context| context.session_id)
            .map(PluginScopeKey::session);
        self.lookup_tool_for_name_scope(name, scope.as_ref())
    }

    pub fn lookup_tool_for_name_scope(
        &self,
        name: &str,
        scope: Option<&PluginScopeKey>,
    ) -> Option<RegisteredTool> {
        if name.parse::<ToolKey>().is_ok()
            && let Some(tool) = self.lookup_tool_for_scope(name, scope)
        {
            return Some(tool);
        }

        let mut matches = self
            .registered_tools_for_scope(scope)
            .into_iter()
            .filter(|tool| registered_tool_matches_name(tool, name));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    pub fn lookup_tool_for_scope(
        &self,
        canonical_name: &str,
        scope: Option<&PluginScopeKey>,
    ) -> Option<RegisteredTool> {
        let key: ToolKey = match canonical_name.parse() {
            Ok(key) => key,
            Err(error) => {
                tracing::warn!(
                    canonical_name,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "parse canonical tool name for lookup",
                        &error,
                    ),
                    "plugin tool lookup rejected an invalid canonical name"
                );
                return None;
            }
        };
        if let Some(value) = self
            ._host_handle
            .scoped_tool_registry()
            .resolve(scope, &key)
        {
            return Some(value.value);
        }
        plugin_tool_registry_read(&self.tool_registry, "look up plugin tool")
            .lookup_tool_by_key(&key)
            .cloned()
    }

    pub fn registered_tools(&self) -> Vec<RegisteredTool> {
        let scope = host_api::current_host_callback_context()
            .and_then(|context| context.session_id)
            .map(PluginScopeKey::session);
        self.registered_tools_for_scope(scope.as_ref())
    }

    pub fn registered_tools_for_scope(
        &self,
        scope: Option<&PluginScopeKey>,
    ) -> Vec<RegisteredTool> {
        let mut tools = plugin_tool_registry_read(&self.tool_registry, "list registered tools")
            .registered_tools()
            .map(|tool| (tool.tool_key().clone(), tool.clone()))
            .collect::<BTreeMap<_, _>>();
        for (key, value) in self._host_handle.scoped_tool_registry().visible(scope) {
            tools.insert(key, value.value);
        }
        tools.into_values().collect()
    }

    pub fn tool_registry_snapshot(&self) -> crate::registry::ToolRegistrySnapshot {
        let scope = host_api::current_host_callback_context()
            .and_then(|context| context.session_id)
            .map(PluginScopeKey::session);
        crate::registry::ToolRegistrySnapshot {
            generation: self.tool_registry_generation(),
            tools: self.registered_tools_for_scope(scope.as_ref()),
        }
    }

    pub fn tool_registry_generation(&self) -> u64 {
        plugin_tool_registry_read(&self.tool_registry, "read tool registry generation").generation()
    }

    pub fn status_registry(&self) -> Arc<crate::status::StatusRegistry> {
        Arc::clone(&self.statuses)
    }

    pub fn plugin_status(&self, plugin_id: &str) -> Option<crate::status::PluginStatus> {
        let plugin_key = parse_plugin_key_for_lookup(plugin_id, "read plugin status")?;
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
        let plugin_key = parse_plugin_key_for_lookup(plugin_id.as_str(), "append plugin log")?;
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

    pub fn architecture_catalog(&self) -> PluginArchitectureCatalog {
        let mut plugins = Vec::new();
        let mut dependencies = Vec::new();
        let mut effects = Vec::new();

        for (plugin_id, configured) in &self.configured_plugins {
            let Ok(plugin_key) = plugin_id.parse::<PluginKey>() else {
                continue;
            };
            let Some(status) = self.plugin_status(plugin_id) else {
                continue;
            };
            let blocked =
                self.activation_blocks
                    .get(plugin_id)
                    .map(|block| PluginActivationDiagnostic {
                        code: block.code.to_string(),
                        message: block.message.clone(),
                        dependencies: block
                            .dependencies
                            .iter()
                            .filter_map(|dependency| dependency.parse().ok())
                            .collect(),
                    });
            let manifest = self.prefetched_manifests.get(plugin_id);
            plugins.push(PluginArchitectureNode {
                plugin_id: plugin_key.clone(),
                enabled: configured.enabled,
                status,
                activation_epoch: self
                    .activation_epochs
                    .get(plugin_id)
                    .map(|epoch| format!("{epoch:016x}")),
                blocked,
                service_exports: manifest
                    .map(|manifest| manifest.services.exports.clone())
                    .unwrap_or_default(),
                service_imports: manifest
                    .map(|manifest| manifest.services.imports.clone())
                    .unwrap_or_default(),
            });

            dependencies.extend(configured.activation.required_keys().cloned().map(
                |provider_id| PluginDependencyEdge {
                    consumer_id: plugin_key.clone(),
                    provider_id,
                    kind: PluginDependencyKind::Explicit,
                    service_id: None,
                    api_version: None,
                },
            ));
            dependencies.extend(
                self.service_bindings
                    .iter()
                    .filter(|(key, _)| key.consumer == *plugin_id)
                    .filter_map(|(_, binding)| {
                        Some(PluginDependencyEdge {
                            consumer_id: plugin_key.clone(),
                            provider_id: parse_plugin_key_for_lookup(
                                binding.provider.as_str(),
                                "inspect plugin service dependency",
                            )?,
                            kind: if binding.optional {
                                PluginDependencyKind::OptionalService
                            } else {
                                PluginDependencyKind::RequiredService
                            },
                            service_id: Some(binding.service.clone()),
                            api_version: Some(binding.api_version),
                        })
                    }),
            );
            if let Some(scope) = self._host_handle.effect_scope_inspect(&plugin_key) {
                effects.extend(
                    scope
                        .effects
                        .into_iter()
                        .map(|effect| PluginArchitectureEffect {
                            plugin_id: plugin_key.clone(),
                            effect,
                        }),
                );
            }
        }

        plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
        dependencies.sort_by(|left, right| {
            left.consumer_id
                .cmp(&right.consumer_id)
                .then(left.provider_id.cmp(&right.provider_id))
                .then(left.service_id.cmp(&right.service_id))
        });
        dependencies.dedup();
        effects.sort_by(|left, right| {
            left.plugin_id
                .cmp(&right.plugin_id)
                .then(left.effect.id.cmp(&right.effect.id))
        });
        let mut pipelines = vec![
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "tool.before".to_string(),
                    mode: crate::event_pipeline::PluginEventMode::TransformBail,
                    durable: false,
                    scoped: true,
                },
                failure_policy: Some(crate::event_pipeline::PluginPipelineFailurePolicy::Abort),
                handlers: self.tool_before_pipeline.inventory(),
            },
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "tool.after".to_string(),
                    mode: crate::event_pipeline::PluginEventMode::Transform,
                    durable: false,
                    scoped: true,
                },
                failure_policy: Some(crate::event_pipeline::PluginPipelineFailurePolicy::Abort),
                handlers: self.tool_after_pipeline.inventory(),
            },
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "command.invoke".to_string(),
                    mode: crate::event_pipeline::PluginEventMode::Around,
                    durable: false,
                    scoped: true,
                },
                failure_policy: None,
                handlers: self.command_pipeline.inventory(),
            },
        ];
        pipelines.extend([
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "model.request.before".into(),
                    mode: crate::event_pipeline::PluginEventMode::Guard,
                    durable: false,
                    scoped: true,
                },
                failure_policy: None,
                handlers: self.lifecycle.model_before.inventory(),
            },
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "model.request.after".into(),
                    mode: crate::event_pipeline::PluginEventMode::ParallelObserve,
                    durable: false,
                    scoped: true,
                },
                failure_policy: None,
                handlers: self.lifecycle.model_after.inventory(),
            },
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "model.request.error".into(),
                    mode: crate::event_pipeline::PluginEventMode::Bail,
                    durable: false,
                    scoped: true,
                },
                failure_policy: None,
                handlers: self.lifecycle.model_error.inventory(),
            },
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "compaction.before".into(),
                    mode: crate::event_pipeline::PluginEventMode::TransformBail,
                    durable: false,
                    scoped: true,
                },
                failure_policy: Some(crate::event_pipeline::PluginPipelineFailurePolicy::Abort),
                handlers: self.lifecycle.compaction_before.inventory(),
            },
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "compaction.after".into(),
                    mode: crate::event_pipeline::PluginEventMode::ParallelObserve,
                    durable: false,
                    scoped: true,
                },
                failure_policy: None,
                handlers: self.lifecycle.compaction_after.inventory(),
            },
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "subagent.start".into(),
                    mode: crate::event_pipeline::PluginEventMode::Guard,
                    durable: false,
                    scoped: true,
                },
                failure_policy: None,
                handlers: self.lifecycle.subagent_start.inventory(),
            },
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "subagent.stop".into(),
                    mode: crate::event_pipeline::PluginEventMode::ParallelObserve,
                    durable: false,
                    scoped: true,
                },
                failure_policy: None,
                handlers: self.lifecycle.subagent_stop.inventory(),
            },
            PluginArchitecturePipeline {
                definition: crate::event_pipeline::PluginEventDefinition {
                    id: "permission.request".into(),
                    mode: crate::event_pipeline::PluginEventMode::Guard,
                    durable: false,
                    scoped: true,
                },
                failure_policy: None,
                handlers: self.lifecycle.permission_request.inventory(),
            },
        ]);
        PluginArchitectureCatalog {
            profiles: self.profile_resolution.clone(),
            reload: self.reload_plan.clone(),
            plugins,
            dependencies,
            effects,
            pipelines,
            tool_registrations: self._host_handle.scoped_tool_registry().inspect(),
            command_registrations: self.command_registry.inspect(),
        }
    }

    pub fn plugin_inspect(&self, plugin_id: &str) -> Option<PluginInspect> {
        let status = self.plugin_status(plugin_id)?;
        let plugin_key = parse_plugin_key_for_lookup(plugin_id, "inspect plugin")?;
        let plugin = self.plugins_by_id.get(&plugin_key);
        let manifest = plugin
            .as_ref()
            .map(|plugin| plugin.manifest.clone())
            .or_else(|| self.prefetched_manifests.get(plugin_id).cloned());
        let authority = plugin.map(|plugin| plugin.authority_summary());
        let configured_plugin = self.configured_plugins.get(plugin_id).cloned().or_else(|| {
            plugin
                .as_ref()
                .map(|plugin| plugin.configured_plugin.clone())
        });
        let hooks = plugin
            .as_ref()
            .map(|plugin| match hook_registration_for_plugin(plugin) {
                Ok(registration) => vec![registration],
                Err(error) => {
                    tracing::error!(
                        plugin = %plugin.key(),
                        diagnostic = %error.diagnostic_message(),
                        "loaded plugin has invalid hook documentation",
                    );
                    Vec::new()
                }
            })
            .unwrap_or_default();
        let activation = configured_plugin.as_ref().map(|configured| {
            let blocked =
                self.activation_blocks
                    .get(plugin_id)
                    .map(|block| PluginActivationDiagnostic {
                        code: block.code.to_string(),
                        message: block.message.clone(),
                        dependencies: block
                            .dependencies
                            .iter()
                            .filter_map(|dependency| dependency.parse().ok())
                            .collect(),
                    });
            PluginActivationInspect {
                requires: configured.activation.required_keys().cloned().collect(),
                after: configured.activation.after.clone(),
                blocked,
            }
        });
        let services = manifest.as_ref().map(|manifest| {
            let imports = manifest
                .services
                .imports
                .iter()
                .cloned()
                .map(|declaration| {
                    let key = PluginServiceBindingKey {
                        consumer: plugin_id.to_string(),
                        service: declaration.id.clone(),
                        api_version: declaration.api_version,
                    };
                    let binding = self.service_bindings.get(&key);
                    let resolved_provider = binding.map(|binding| binding.provider.clone());
                    let methods = binding
                        .map(|binding| binding.methods.values().cloned().collect())
                        .unwrap_or_default();
                    let state = if resolved_provider.is_some() {
                        "bound"
                    } else if declaration.optional {
                        "unbound_optional"
                    } else if self
                        .activation_blocks
                        .get(plugin_id)
                        .is_some_and(|block| block.code.starts_with("service_"))
                    {
                        "blocked"
                    } else {
                        "unresolved"
                    };
                    PluginServiceImportInspect {
                        declaration,
                        resolved_provider,
                        methods,
                        state: state.to_string(),
                    }
                })
                .collect();
            PluginServiceInspect {
                exports: manifest.services.exports.clone(),
                imports,
            }
        });
        let effects = self._host_handle.effect_scope_inspect(&plugin_key);
        Some(PluginInspect {
            status,
            manifest,
            authority,
            hooks,
            configured_plugin,
            activation,
            services,
            effects,
        })
    }

    /// Native asynchronous `tool.before` dispatch.
    ///
    /// Handlers are registered once during Host construction, owned by their
    /// plugin effect scopes, and receive the latest transformed value. The
    /// first abort or transport/plugin error terminates later handlers.
    pub async fn dispatch_tool_before(
        &self,
        input: ToolBeforeInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<ToolBeforeInput, PluginError> {
        let report = self
            .tool_before_pipeline
            .dispatch(ToolBeforeDispatch {
                input,
                cancellation,
            })
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        match report.outcome {
            crate::event_pipeline::PluginTransformBailOutcome::Continue(dispatch) => {
                Ok(dispatch.input)
            }
            crate::event_pipeline::PluginTransformBailOutcome::Bail(ToolBeforeBail::Abort(
                reason,
            )) => Err(PluginError::internal(reason)),
            crate::event_pipeline::PluginTransformBailOutcome::Bail(ToolBeforeBail::Error(
                error,
            )) => Err(error),
        }
    }

    /// Native asynchronous `tool.after` dispatch for runtime request paths.
    pub async fn dispatch_tool_after(
        &self,
        input: ToolAfterInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<ToolAfterInput, PluginError> {
        let report = self
            .tool_after_pipeline
            .dispatch(ToolAfterDispatch {
                input,
                cancellation,
            })
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        Ok(report.value.input)
    }

    /// Invoke only the plugin that owns `registered_tool`. Rendering is a
    /// pure read-time projection; `None` means the plugin delegates to the
    /// host fallback chain.
    pub async fn render_tool(
        &self,
        registered_tool: &RegisteredTool,
        mut input: crate::sdk::ToolRenderInput,
    ) -> Result<Option<crate::sdk::ToolRenderOutput>, PluginError> {
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
        input.tool_name = registered_tool.tool_name().to_owned();
        let params = serde_json::to_value(&input)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let value = self
            .call_hook_with_default_context(
                &plugin,
                method::HOOK_TOOL_RENDER,
                params,
                self.timeouts.fast_or(Duration::from_secs(2)),
            )
            .await
            .map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|error| PluginError::invalid_params_error(&error))
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
        registered_tool
            .validate_input(&input.input)
            .map_err(PluginError::invalid_params)?;
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
        let tool_name = registered_tool.tool_name().to_string();
        let mcp_app =
            host_api::mcp_app_request_binding(&registered_tool.plugin_full_name(), &input)?;
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params_error(&e))?;
        let plugin_key = plugin.key();
        let invoke = Box::pin(self._host_handle.run_in_authorized_callback_context(
            &plugin_key,
            HostCallbackContext {
                mcp_app,
                caller: input.caller.clone(),
                session_id: Some(session_id),
                call_id: Some(call_id),
                workspace_root: Some(workspace_root),
                tool_name: Some(tool_name),
                ..Default::default()
            },
            call_with_timeout(&plugin, method::HOOK_TOOL_INVOKE, params, timeout),
        ));
        let result = match cancellation {
            Some(cancellation) => tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(TransportError::Cancelled),
                result = invoke => result,
            },
            None => invoke.await,
        };
        let value = result.map_err(transport_to_plugin_error)?;
        let output: ToolInvokeOutput =
            serde_json::from_value(value).map_err(crate::schema::output_contract_error)?;
        registered_tool
            .validate_output(output.payload.as_ref())
            .map_err(crate::schema::output_contract_error)?;
        Ok(output)
    }

    pub fn register_command_middleware<F, Fut>(
        &self,
        owner_plugin_id: &str,
        priority: i32,
        label: impl Into<String>,
        handler: F,
    ) -> Result<crate::event_pipeline::PluginPipelineRegistration, PluginError>
    where
        F: Fn(
                CommandDispatch,
                crate::event_pipeline::PluginAroundNext<CommandDispatch, CommandResult, PluginError>,
            ) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = Result<CommandResult, PluginError>> + Send + 'static,
    {
        let owner: PluginKey = owner_plugin_id.parse().map_err(|error| {
            PluginError::invalid_params(format!("invalid middleware owner plugin id: {error}"))
        })?;
        let scope = self._host_handle.effect_scope(&owner).ok_or_else(|| {
            PluginError::not_implemented(format!(
                "plugin `{owner_plugin_id}` has no active effect scope"
            ))
        })?;
        self.command_pipeline
            .register(&scope, priority, label, handler)
            .map_err(|error| PluginError::internal_error(&error))
    }

    pub async fn invoke_imported_skill_fork(
        &self,
        source: &PluginKey,
        mut input: CommandInvokeInput,
        scope: crate::agent_plugin::SkillFork,
        prompt: String,
    ) -> Result<CommandResult, PluginError> {
        use crate::agent_plugin::{AgentPluginFormat, FORKED_SKILL_COMMAND, ForkedSkillInvocation};

        let plugin = self
            .plugins_by_id
            .get(source)
            .filter(|plugin| {
                plugin.agent_plugin().is_some_and(|package| {
                    package.format == AgentPluginFormat::ClaudeCode
                        && package.skill_dirs.contains(&scope.directory)
                })
            })
            .ok_or_else(|| {
                PluginError::invalid_params("Claude fork does not belong to an active package")
            })?;
        input.command_id = FORKED_SKILL_COMMAND.into();
        input.input = serde_json::to_value(ForkedSkillInvocation { scope, prompt })
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let context = HostCallbackContext {
            session_id: input.session_id,
            call_id: input.call_id,
            workspace_root: input.workspace_root.clone(),
            ..Default::default()
        };
        let params = serde_json::to_value(input)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let call = self._host_handle.run_in_authorized_callback_context(
            source,
            context,
            call_with_timeout(
                plugin,
                method::COMMAND_INVOKE,
                params,
                Duration::from_secs(190),
            ),
        );
        let value =
            await_transport_with_cancellation(crate::sdk::current_cancellation_token(), call)
                .await
                .map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|error| PluginError::invalid_params_error(&error))
    }

    pub async fn invoke_plugin_command_async(
        &self,
        plugin_id: &str,
        input: CommandInvokeInput,
    ) -> Result<CommandResult, PluginError> {
        self.invoke_command(plugin_id, input, None).await
    }

    pub fn imported_prompt_command_name(
        &self,
        plugin: &PluginKey,
        command_id: &str,
    ) -> Option<String> {
        let package = self.plugins_by_id.get(plugin)?.agent_plugin()?;
        package
            .has_inline_prompt_command(command_id)
            .then(|| format!("{}:{command_id}", package.name))
    }

    pub fn command_accepts_attachments(&self, plugin: &PluginKey, command_id: &str) -> bool {
        self.plugins_by_id
            .get(plugin)
            .and_then(|plugin| plugin.agent_plugin())
            .is_some_and(|package| package.command_accepts_attachments(command_id))
    }

    /// Render a prompt command from within the authorized user submission.
    /// The submission owns the native and classic prompt guards.
    pub async fn invoke_imported_prompt_command(
        &self,
        plugin: &PluginKey,
        input: CommandInvokeInput,
        enabled: impl Fn(&PluginKey) -> bool + Sync,
    ) -> Result<CommandResult, PluginError> {
        if input.session_id.is_none()
            || input.call_id.is_some()
            || self
                .imported_prompt_command_name(plugin, &input.command_id)
                .is_none()
            || !enabled(plugin)
        {
            return Err(PluginError::invalid_params(
                "The prompt command is unavailable in this session",
            ));
        }
        self.invoke_command(&plugin.to_string(), input, Some(&enabled))
            .await
    }

    async fn invoke_command(
        &self,
        plugin_id: &str,
        mut input: CommandInvokeInput,
        submission_plugins: Option<&(dyn Fn(&PluginKey) -> bool + Sync)>,
    ) -> Result<CommandResult, PluginError> {
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
        let definition = self
            .resolve_command_for_session(input.session_id, plugin_id, input.command_id.as_str())
            .ok_or_else(|| {
                PluginError::not_implemented(format!(
                    "plugin `{plugin_id}` does not declare command `{}`",
                    input.command_id
                ))
            })?;
        if !matches!(&definition.target, crate::sdk::CommandTarget::Method { .. }) {
            return Err(PluginError::invalid_params(format!(
                "command `{}` is tool-backed and must execute through the Runtime command resolver",
                input.command_id
            )));
        }
        if input.cordis_attachments.is_some()
            && (input.session_id.is_none()
                || input.call_id.is_some()
                || !self.command_accepts_attachments(&plugin_key, &input.command_id))
        {
            return Err(PluginError::invalid_params(
                "This command does not accept composer attachments in this invocation",
            ));
        }
        input.input = super::decode_plugin_command_input(&definition, input.input, &input.raw)?;

        let timeout = self.timeouts.fast_or(Duration::from_secs(10));
        let timeout = if plugin.agent_plugin().is_some()
            && matches!(&definition.target, crate::sdk::CommandTarget::Method { handler }
                if handler == crate::agent_plugin::DELEGATED_COMMAND_HANDLER)
        {
            timeout.max(Duration::from_secs(300))
        } else if plugin.agent_plugin().is_some()
            && matches!(&definition.target, crate::sdk::CommandTarget::Method { handler }
                if handler == crate::agent_plugin::CORDIS_MCP_COMMAND_HANDLER)
        {
            timeout.max(Duration::from_secs(90))
        } else {
            timeout
        };
        let contract = definition.input.clone();
        input.prompt_context.clear();
        let injections = plugin.agent_plugin().map_or(0, |package| {
            package.command_shell_injection_count(&input.command_id)
        });
        let timeout =
            timeout.saturating_add(Duration::from_secs(130).saturating_mul(injections as u32));
        if submission_plugins.is_none()
            && injections > 0
            && input.call_id.is_none()
            && let Some(session_id) = input.session_id
            && self.has_subscription(HookSubscription::USER_PROMPT_SUBMIT)
        {
            let slash = definition.slash.as_deref().unwrap_or(&input.command_id);
            let args = if !input.raw.is_empty()
                && contract
                    .parse_shorthand(&input.raw)
                    .as_ref()
                    .is_ok_and(|parsed| parsed == &input.input)
            {
                &input.raw
            } else {
                input.input["args"].as_str().unwrap_or_default()
            };
            let prompt = if args.is_empty() {
                slash.to_owned()
            } else {
                format!("{slash} {args}")
            };
            let updated = match self
                .dispatch_user_prompt_submit(UserPromptSubmitInput {
                    session_id,
                    prompt: prompt.clone(),
                })
                .await
            {
                Ok(updated) => updated.prompt,
                Err(error) => {
                    return Ok(CommandResult::failed("Prompt blocked by hook")
                        .with_detail(error.failure.user.fallback));
                }
            };
            if updated != prompt {
                if let Some(context) = updated
                    .strip_prefix(&prompt)
                    .and_then(|text| text.strip_prefix('\n'))
                {
                    input.prompt_context = context.to_owned();
                } else {
                    return Ok(CommandResult::succeeded("Command prompt rewritten by hook")
                        .with_effect(crate::sdk::CommandHostEffect::InsertPrompt {
                            prompt: updated,
                        }));
                }
            }
        }
        let expansion = if input.session_id.is_some()
            && input.call_id.is_none()
            && let Some(package) = plugin.agent_plugin()
            && package.has_prompt_command(&input.command_id)
            && self.has_subscription(HookSubscription::USER_PROMPT_EXPANSION)
        {
            Some(Arc::new(PromptExpansionHooks {
                package_name: package.name.clone(),
                hooks: self
                    .plugins
                    .iter()
                    .filter(|plugin| plugin.subscribes(HookSubscription::USER_PROMPT_EXPANSION))
                    .filter(|plugin| {
                        submission_plugins.is_none_or(|enabled| enabled(&plugin.key()))
                    })
                    .cloned()
                    .collect(),
                runs: Arc::clone(&self.hook_runs),
                timeout: self.timeouts.fast_or(Duration::from_secs(5)),
            }))
        } else {
            None
        };
        let cordis_command = submission_plugins.is_none()
            && plugin.agent_plugin().is_some()
            && matches!(&definition.target, crate::sdk::CommandTarget::Method { handler }
                if handler == crate::agent_plugin::CORDIS_COMMAND_HANDLER
                    || handler == crate::agent_plugin::CORDIS_MCP_COMMAND_HANDLER);
        let dispatch = CommandDispatch::new(plugin_key.clone(), input);
        let host_handle = Arc::clone(&self._host_handle);
        let result = self
            .command_pipeline
            .dispatch(dispatch, move |dispatch| {
                let plugin = Arc::clone(&plugin);
                let contract = contract.clone();
                let host_handle = Arc::clone(&host_handle);
                let expansion = expansion.clone();
                async move {
                    let plugin_key = dispatch.plugin_id().clone();
                    let command_id = dispatch.command_id().to_string();
                    let mut input = dispatch.into_input();
                    contract
                        .validate_value(&input.input)
                        .map_err(|error| PluginError::invalid_params_error(&error))?;
                    if let Some(expansion) = expansion
                        && let Err(reason) = expansion.apply(&host_handle, &mut input).await
                    {
                        return Ok(CommandResult::failed("Prompt expansion blocked by hook")
                            .with_detail(reason));
                    }
                    let session_id = input.session_id;
                    let call_id = input.call_id;
                    let workspace_root = input.workspace_root.clone();
                    let plugin_command = (cordis_command && call_id.is_none()
                        && session_id.is_some_and(|session| session > 0))
                        .then(|| crate::sdk::host_api::PluginCommandInvocation {
                            id: agena_domain::ExecutionId::new(),
                            command_id: command_id.clone(),
                        });
                    let params = serde_json::to_value(&input)
                        .map_err(|error| PluginError::invalid_params_error(&error))?;
                    let value = host_handle
                        .run_in_authorized_callback_context(
                            &plugin_key,
                            HostCallbackContext {
                                session_id,
                                call_id,
                                workspace_root,
                                plugin_command,
                                tool_name: None,
                                ..Default::default()
                            },
                            call_with_timeout(&plugin, method::COMMAND_INVOKE, params, timeout),
                        )
                        .await
                        .map_err(transport_to_plugin_error)?;
                    let result: CommandResult = serde_json::from_value(value).map_err(|error| {
                        PluginError::invalid_params(format!(
                            "invalid command result for `{command_id}`: {error}"
                        ))
                    })?;
                    result.validate().map_err(|error| {
                        PluginError::invalid_params(format!(
                            "invalid command result for `{command_id}`: {error}"
                        ))
                    })?;
                    Ok(result)
                }
            })
            .await?;
        result.validate().map_err(|error| {
            PluginError::invalid_params(format!(
                "command middleware produced an invalid result for `{}`: {error}",
                definition.id
            ))
        })?;
        Ok(result)
    }

    pub(super) fn tool_invoke_timeout(&self, registered_tool: &RegisteredTool) -> Duration {
        let base = self.timeouts.tool_invoke_or(Duration::from_secs(300));
        // Interactive and agent tools need long-lived budgets; these are
        // declared on the tool's tags, not static capabilities.
        let tags = registered_tool.effective_tags();
        if crate::sdk::ToolTag::is_interactive(&tags) || crate::sdk::ToolTag::is_agent(&tags) {
            return base.max(Duration::from_secs(60 * 60 * 24));
        }
        base
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
        registered_tool
            .validate_input(&input.input)
            .map_err(PluginError::invalid_params)?;
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

        let mut context = tool_hook_context(
            &plugin,
            &input.tool_name,
            Some(input.session_id),
            Some(input.call_id),
            Some(input.workspace_root.clone()),
        );
        context.caller = input.caller.clone();
        let (context, callback_authority) = self
            ._host_handle
            .issue_callback_authority(&plugin.key(), context);
        if let Some(stream) = callback_authority
            .completion
            .completion
            .scope(host_api::run_in_isolated_host_callback_context(
                context.clone(),
                plugin.transport.invoke_stream(input.clone()),
            ))
            .await
            .map_err(transport_to_plugin_error)?
        {
            let stream_id = stream.stream_id;
            let chunks = stream.chunks;
            let native_end = stream.end;
            let (mut end_tx, end_rx) = tokio::sync::oneshot::channel();
            let stream_id_for_task = stream_id.clone();
            let definition = registered_tool.clone();
            tokio::spawn(async move {
                // Keep callback authority until the transport reports a
                // terminal stream result; chunk consumers may stop earlier.
                let _callback_authority = callback_authority;
                let result = tokio::select! {
                    biased;
                    _ = end_tx.closed() => return,
                    result = native_end => result.unwrap_or_else(|error| {
                        Err(PluginError::internal(format!(
                            "plugin stream ended without a terminal result: {error}"
                        )))
                    }),
                };
                let result = result.and_then(|output| {
                    definition
                        .validate_output(output.payload.as_ref())
                        .map_err(crate::schema::output_contract_error)?;
                    Ok(output)
                });
                if end_tx.send(result).is_err() {
                    tracing::debug!(
                        stream_id = %stream_id_for_task,
                        "plugin transport stream terminal-result receiver was dropped"
                    );
                }
            });
            return Ok(ToolInvokeStream {
                stream_id,
                chunks,
                end: end_rx,
            });
        }

        let timeout = self.tool_invoke_timeout(registered_tool);
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params_error(&e))?;
        let invoke_result = callback_authority
            .completion
            .completion
            .scope(host_api::run_in_isolated_host_callback_context(
                context,
                call_with_timeout(&plugin, method::HOOK_TOOL_INVOKE, params, timeout),
            ))
            .await
            .map_err(transport_to_plugin_error)?;
        let result: ToolInvokeOutput =
            serde_json::from_value(invoke_result).map_err(crate::schema::output_contract_error)?;
        registered_tool
            .validate_output(result.payload.as_ref())
            .map_err(crate::schema::output_contract_error)?;

        let (tx, rx) = tokio::sync::mpsc::channel::<ToolStreamChunk>(8);
        let (end_tx, end_rx) = tokio::sync::oneshot::channel();
        let stream_id = format!("emu-{}", uuid::Uuid::new_v4().simple());
        drop(tx);
        if end_tx
            .send(Ok(ToolStreamEnd::from_output(stream_id.clone(), result)))
            .is_err()
        {
            tracing::debug!(
                stream_id,
                "emulated plugin stream terminal-result receiver was dropped"
            );
        }
        Ok(ToolInvokeStream {
            stream_id,
            chunks: rx,
            end: end_rx,
        })
    }

    pub async fn dispatch_tool_batch_after(
        &self,
        input: crate::sdk::ToolBatchInput,
        cancellation: tokio_util::sync::CancellationToken,
        is_enabled: impl Fn(&PluginKey) -> bool,
    ) -> Option<String> {
        use crate::sdk::ToolBatchPatch;

        let event = "tool.batch.after";
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        let timeout = self.timeouts.chat_or(Duration::from_secs(30));
        let mut blocked = None;
        for plugin in &self.plugins {
            let key = plugin.key();
            if !plugin.subscribes(HookSubscription::TOOL_BATCH_AFTER) || !is_enabled(&key) {
                continue;
            }
            let result = await_transport_with_cancellation(
                Some(cancellation.clone()),
                self._host_handle.run_in_authorized_callback_context(
                    &key,
                    HostCallbackContext {
                        session_id: session,
                        ..Default::default()
                    },
                    call_with_timeout(
                        plugin,
                        method::HOOK_TOOL_BATCH_AFTER,
                        params.clone(),
                        timeout,
                    ),
                ),
            )
            .await;
            let raw = match result {
                Ok(value) => value,
                Err(_) if cancellation.is_cancelled() => break,
                Err(error) => {
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        event,
                        &key.to_string(),
                        session,
                        &error,
                    )]);
                    continue;
                }
            };
            let patch = match serde_json::from_value::<Option<ToolBatchPatch>>(raw) {
                Ok(Some(patch)) => patch,
                Ok(None) => continue,
                Err(error) => {
                    self.push_hook_runs(vec![HookRunRecord::new(
                        event,
                        key.to_string(),
                        session,
                        HookRunStatus::Failed,
                        "invalid tool batch patch",
                        Some(error.to_string()),
                    )]);
                    continue;
                }
            };
            let block = patch
                .block_reason
                .filter(|reason| !reason.trim().is_empty())
                .map(|reason| reason.chars().take(8192).collect::<String>());
            let context = patch
                .additional_context
                .filter(|text| !text.trim().is_empty())
                .map(|text| text.chars().take(16384).collect::<String>());
            if let Some(reason) = &block {
                blocked.get_or_insert_with(|| reason.clone());
            }
            let summary = if block.is_some() {
                "tool batch hook stopped the turn"
            } else {
                "tool batch hook added context"
            };
            if let Some(message) = block.or(context) {
                self.push_hook_runs(vec![
                    HookRunRecord::new(
                        event,
                        key.to_string(),
                        session,
                        HookRunStatus::Applied,
                        summary,
                        None,
                    )
                    .with_message(Some(message)),
                ]);
            }
        }
        blocked
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
                self.call_session_hook(
                    plugin,
                    method::HOOK_SHELL_ENV,
                    params,
                    timeout,
                    input.session_id,
                ),
            )
            .await
            .map_err(transport_to_plugin_error)?;
            if matches!(&result, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<ShellEnvPatch> = serde_json::from_value(result)
                .map_err(|error| PluginError::invalid_params_error(&error))?;
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
        let result = self
            .chain_patch_with_default_context::<ChatMessageInput, ChatMessagePatch, _>(
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
        self.push_hook_runs(runs);
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
            self.chain_patch_with_default_context::<ChatParamsInput, ChatParamsPatch, _>(
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
        let session_id = Some(input.session_id);
        let mut runs = Vec::new();
        let result = await_transport_with_cancellation(
            cancellation,
            dispatcher::chain_patch_in_context::<ChatHeadersInput, ChatHeadersPatch, _, _>(
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
                |plugin, input| {
                    Some(HostCallbackContext {
                        plugin_id: Some(plugin.key().to_string()),
                        session_id: Some(input.session_id),
                        ..Default::default()
                    })
                },
                Some(&self._host_handle),
                session_id,
                &mut runs,
            ),
        )
        .await;
        self.push_hook_runs(runs);
        result.map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_chat_system_transform(
        &self,
        input: ChatSystemTransformInput,
    ) -> Result<ChatSystemTransformInput, PluginError> {
        self.dispatch_chat_system_transform_for(input, |_| true).await
    }

    pub async fn dispatch_chat_system_transform_for(
        &self,
        input: ChatSystemTransformInput,
        cordis_enabled: impl Fn(&PluginKey) -> bool,
    ) -> Result<ChatSystemTransformInput, PluginError> {
        let plugins = self
            .plugins
            .iter()
            .filter(|plugin| {
                plugin.agent_plugin().is_none_or(|package| {
                    package.format != crate::agent_plugin::AgentPluginFormat::DeepSeekHarness
                        || cordis_enabled(&plugin.key())
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        let session_id = Some(input.session_id);
        let mut runs = Vec::new();
        let result = dispatcher::chain_patch_in_context::<
            ChatSystemTransformInput,
            ChatSystemTransformPatch,
            _,
            _,
        >(
            &plugins,
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
            Some(&self._host_handle),
            session_id,
            &mut runs,
        )
        .await;
        self.push_hook_runs(runs);
        result.map_err(transport_to_plugin_error)
    }

    pub fn has_imported_notification_hooks(&self) -> bool {
        self.plugins.iter().any(|plugin| {
            plugin.agent_plugin().is_some() && plugin.subscribes(HookSubscription::NOTIFICATION)
        })
    }

    pub async fn broadcast_imported_notification(
        &self,
        input: NotificationInput,
        is_enabled: impl Fn(&PluginKey) -> bool,
    ) {
        let Some(session) = input.session_id else {
            return;
        };
        let params = serde_json::json!(input);
        let notifications = self
            .plugins
            .iter()
            .filter(|plugin| {
                plugin.agent_plugin().is_some()
                    && plugin.subscribes(HookSubscription::NOTIFICATION)
                    && is_enabled(&plugin.key())
            })
            .map(|plugin| {
                let params = params.clone();
                async move {
                    let key = plugin.key();
                    let result = self
                        ._host_handle
                        .run_in_authorized_callback_context(
                            &key,
                            HostCallbackContext {
                                session_id: Some(session),
                                ..Default::default()
                            },
                            call_with_timeout(
                                plugin,
                                method::HOOK_NOTIFICATION,
                                params,
                                Duration::from_secs(5),
                            ),
                        )
                        .await;
                    if let Err(error) = result {
                        self.push_hook_runs(vec![dispatcher::transport_failure_record(
                            "notification",
                            &key.to_string(),
                            Some(session),
                            &error,
                        )]);
                    }
                }
            });
        futures_util::future::join_all(notifications).await;
    }

    pub async fn broadcast_notification(&self, input: NotificationInput) {
        let timeout = Duration::from_secs(5);
        let mut notifications = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::NOTIFICATION) || plugin.agent_plugin().is_some()
            {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            notifications.push(async move {
                let plugin_id = plugin.key();
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(error) => {
                        tracing::error!(
                            plugin_id = %plugin_id,
                            hook = method::HOOK_NOTIFICATION,
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to serialize plugin notification hook payload",
                                &error,
                            ),
                            "plugin notification hook payload serialization failed"
                        );
                        return;
                    }
                };
                // One-way: `notify` returns when the frame is written, not
                // when the plugin finishes handling it. A callback authority
                // minted here would be revoked before the handler could use
                // it, so this hook deliberately carries no callback authority.
                match tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_NOTIFICATION, params),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => tracing::warn!(
                        plugin_id = %plugin_id,
                        hook = method::HOOK_NOTIFICATION,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "plugin notification hook delivery failed",
                            &error,
                        ),
                        "best-effort plugin notification was not delivered"
                    ),
                    Err(error) => tracing::warn!(
                        plugin_id = %plugin_id,
                        hook = method::HOOK_NOTIFICATION,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "plugin notification hook timed out after 5 seconds",
                            &error,
                        ),
                        "best-effort plugin notification timed out"
                    ),
                }
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
                .map_err(|e| PluginError::invalid_params_error(&e))?;
            let context = HostCallbackContext {
                plugin_id: Some(plugin_id.clone()),
                session_id: current.session_id,
                call_id: current.call_id,
                workspace_root: current.workspace_root.clone(),
                ..HostCallbackContext::default()
            };
            let call = call_with_timeout(plugin, method::HOOK_COMMAND_BEFORE, params, timeout);
            let v = match self
                ._host_handle
                .run_in_authorized_callback_context(&plugin.key(), context, call)
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
            let resp: Option<CommandBeforeResponse> =
                serde_json::from_value(v).map_err(|e| PluginError::invalid_params_error(&e))?;
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
            let params =
                serde_json::to_value(&input).map_err(|e| PluginError::invalid_params_error(&e))?;
            let v = match self
                .call_hook_with_default_context(plugin, method::HOOK_PROVIDER_LIST, params, timeout)
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
            let patch: Option<ProviderListPatch> =
                serde_json::from_value(v).map_err(|e| PluginError::invalid_params_error(&e))?;
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
        let result = self
            .chain_patch_with_default_context::<ConfigInput, ConfigPatch, _>(
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
                let plugin_id = plugin.key();
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(error) => {
                        tracing::error!(
                            plugin_id = %plugin_id,
                            hook = method,
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to serialize best-effort plugin hook payload",
                                &error,
                            ),
                            "best-effort plugin hook payload serialization failed"
                        );
                        return;
                    }
                };
                // One-way observer: `notify` completes when the frame is
                // written, so a callback authority minted for it would be
                // revoked before the handler ran. These hooks stay
                // observation-only until a bounded one-way authority exists.
                match tokio::time::timeout(timeout, plugin.transport.notify(method, params)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => tracing::warn!(
                        plugin_id = %plugin_id,
                        hook = method,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "best-effort plugin hook delivery failed",
                            &error,
                        ),
                        "best-effort plugin hook was not delivered"
                    ),
                    Err(error) => tracing::warn!(
                        plugin_id = %plugin_id,
                        hook = method,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "best-effort plugin hook timed out after 5 seconds",
                            &error,
                        ),
                        "best-effort plugin hook timed out"
                    ),
                }
            });
        }
        futures_util::future::join_all(notifications).await;
    }

    // ── session.start ──────────────────────────────────────────────────────

    pub async fn dispatch_session_start(
        &self,
        input: SessionStartInput,
    ) -> Result<SessionStartPatch, PluginError> {
        use crate::sdk::SessionStartSource;

        let imported_only = matches!(
            input.source,
            SessionStartSource::Compact | SessionStartSource::Fork | SessionStartSource::Resume
        );
        let session_id = Some(input.session_id);
        let params = serde_json::to_value(input)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let mut merged = SessionStartPatch::default();
        for plugin in &self.plugins {
            let imported = plugin.agent_plugin().is_some();
            if (imported_only && !imported) || !plugin.subscribes(HookSubscription::SESSION_START) {
                continue;
            }
            let plugin_id = plugin.key().to_string();
            let timeout =
                self.timeouts
                    .fast_or(Duration::from_secs(if imported { 601 } else { 5 }));
            let v = match self
                .call_session_hook(
                    plugin,
                    method::HOOK_SESSION_START,
                    params.clone(),
                    timeout,
                    session_id,
                )
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
                    if imported {
                        continue;
                    }
                    return Err(transport_to_plugin_error(err));
                }
            };
            if matches!(&v, serde_json::Value::Null) {
                // No-op run; only effective runs (and failures) are recorded
                // as transcript activity.
                continue;
            }
            let patch: Option<SessionStartPatch> =
                serde_json::from_value(v).map_err(|e| PluginError::invalid_params_error(&e))?;
            if let Some(p) = patch {
                let stop_reason = p.stop_reason.filter(|_| imported);
                self.push_hook_runs(vec![HookRunRecord::new(
                    "session.start",
                    &plugin_id,
                    session_id,
                    HookRunStatus::Applied,
                    if stop_reason.is_some() {
                        "session.start hook requested a stop"
                    } else {
                        "session.start hook ran"
                    },
                    stop_reason
                        .as_deref()
                        .map(|reason| agena_failure::diagnostic::scrubbed_preserve(reason, 512)),
                )]);
                if let Some(ctx) = p.additional_context {
                    let existing = merged.additional_context.get_or_insert_with(String::new);
                    if !existing.is_empty() {
                        existing.push('\n');
                    }
                    existing.push_str(&ctx);
                }
                if p.initial_user_message.is_some() {
                    merged.initial_user_message = p.initial_user_message;
                }
                if p.session_title.is_some() {
                    merged.session_title = p.session_title;
                }
                merged.stop_reason = merged.stop_reason.or(stop_reason);
                merged.reload_skills |= p.reload_skills;
            }
        }
        Ok(merged)
    }

    pub fn imported_file_watch_filenames(&self) -> impl Iterator<Item = &std::path::PathBuf> {
        self.imported_hook_observers(HookSubscription::SESSION_FILE_CHANGED)
            .filter_map(LoadedPlugin::agent_plugin)
            .flat_map(|package| package.watch_filenames())
    }

    pub(super) fn imported_hook_observers(
        &self,
        subscription: HookSubscription,
    ) -> impl Iterator<Item = &LoadedPlugin> {
        self.plugins.iter().map(Arc::as_ref).filter(move |plugin| {
            plugin.agent_plugin().is_some()
                && plugin.subscribes(subscription)
                && !plugin.effect_scope().cancellation_token().is_cancelled()
        })
    }

    pub async fn dispatch_imported_session_file_changed(
        &self,
        input: crate::sdk::SessionFileChangedInput,
    ) -> Result<Option<Vec<std::path::PathBuf>>, PluginError> {
        use crate::sdk::SessionFileChangedPatch;

        let session_id = Some(input.session_id);
        let params = serde_json::to_value(input)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let calls = self.imported_hook_observers(HookSubscription::SESSION_FILE_CHANGED).map(|plugin| {
            let params = params.clone();
            async move {
                let result = self.call_session_hook(
                    plugin,
                    method::HOOK_SESSION_FILE_CHANGED,
                    params,
                    self.timeouts.fast_or(Duration::from_secs(5)),
                    session_id,
                ).await;
                match result {
                    Ok(raw) => match serde_json::from_value::<Option<SessionFileChangedPatch>>(raw) {
                        Ok(patch) => patch.map(|patch| patch.watch_paths),
                        Err(error) => {
                            tracing::warn!(plugin = %plugin.key(), %error, "invalid FileChanged result");
                            None
                        }
                    },
                    Err(error) => {
                        self.push_hook_runs(vec![dispatcher::transport_failure_record(
                            "session.file.changed", &plugin.key().to_string(), session_id, &error,
                        )]);
                        None
                    }
                }
            }
        });
        Ok(futures_util::future::join_all(calls)
            .await
            .into_iter()
            .flatten()
            .last())
    }

    pub async fn dispatch_imported_turn_completed(
        &self,
        input: crate::sdk::ImportedTurnCompletedInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
    ) {
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        let calls = self
            .imported_hook_observers(HookSubscription::IMPORTED_TURN_COMPLETED)
            .filter(|plugin| enabled(&plugin.key()))
            .map(|plugin| {
                let params = params.clone();
                async move {
                    if let Err(error) = self.call_session_hook(
                        plugin,
                        method::HOOK_IMPORTED_TURN_COMPLETED,
                        params,
                        self.timeouts.fast_or(Duration::from_secs(10)),
                        session,
                    ).await {
                        tracing::warn!(plugin = %plugin.key(), %error, "imported turn completion hook failed");
                    }
                }
            });
        futures_util::future::join_all(calls).await;
    }

    pub async fn dispatch_imported_session_cwd_changed(
        &self,
        input: crate::sdk::SessionCwdChangedInput,
    ) {
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        for plugin in self.imported_hook_observers(HookSubscription::SESSION_CWD_CHANGED) {
            if let Err(error) = self
                .call_session_hook(
                    plugin,
                    method::HOOK_SESSION_CWD_CHANGED,
                    params.clone(),
                    self.timeouts.fast_or(Duration::from_secs(5)),
                    session,
                )
                .await
            {
                self.push_hook_runs(vec![dispatcher::transport_failure_record(
                    "session.cwd.changed",
                    &plugin.key().to_string(),
                    session,
                    &error,
                )]);
            }
        }
    }

    pub async fn dispatch_imported_session_setup(
        &self,
        input: crate::sdk::ImportedSessionSetupInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
    ) {
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        for plugin in self.imported_hook_observers(HookSubscription::IMPORTED_SESSION_SETUP) {
            if !enabled(&plugin.key()) {
                continue;
            }
            if let Err(error) = self
                .call_session_hook(
                    plugin,
                    method::HOOK_IMPORTED_SESSION_SETUP,
                    params.clone(),
                    self.timeouts.fast_or(Duration::from_secs(601)),
                    session,
                )
                .await
            {
                self.push_hook_runs(vec![dispatcher::transport_failure_record(
                    "session.setup",
                    &plugin.key().to_string(),
                    session,
                    &error,
                )]);
            }
        }
    }

    pub async fn broadcast_imported_instructions_loaded(
        &self,
        input: crate::sdk::ImportedInstructionsLoadedInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
    ) {
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        let calls = self
            .imported_hook_observers(HookSubscription::IMPORTED_INSTRUCTIONS_LOADED)
            .filter(|plugin| enabled(&plugin.key()))
            .map(|plugin| {
                let params = params.clone();
                async move {
                    if let Err(error) = self
                        .call_session_hook(
                            plugin,
                            method::HOOK_IMPORTED_INSTRUCTIONS_LOADED,
                            params,
                            self.timeouts.fast_or(Duration::from_secs(30)),
                            session,
                        )
                        .await
                    {
                        self.push_hook_runs(vec![dispatcher::transport_failure_record(
                            "instructions.loaded",
                            &plugin.key().to_string(),
                            session,
                            &error,
                        )]);
                    }
                }
            });
        futures_util::future::join_all(calls).await;
    }

    pub async fn dispatch_imported_task_before(
        &self,
        input: crate::sdk::ImportedTaskBeforeInput,
    ) -> Result<(), PluginError> {
        const EVENT: &str = "task.change.before";
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        for plugin in self.imported_hook_observers(HookSubscription::IMPORTED_TASK_BEFORE) {
            let method = method::HOOK_IMPORTED_TASK_BEFORE;
            let timeout = self.timeouts.fast_or(Duration::from_secs(10));
            let result = self
                .call_session_hook(plugin, method, params.clone(), timeout, session)
                .await;
            match result {
                Ok(result) => {
                    if let Some(reason) = serde_json::from_value::<Option<String>>(result)
                        .map_err(|error| PluginError::invalid_params_error(&error))?
                    {
                        self.push_hook_runs(vec![HookRunRecord::new(
                            EVENT,
                            plugin.key().to_string(),
                            session,
                            HookRunStatus::Applied,
                            "imported hook rejected the task change",
                            Some(reason.clone()),
                        )]);
                        return Err(PluginError::from_kind(
                            crate::sdk::PluginErrorKind::PolicyDenied,
                            reason,
                        ));
                    }
                }
                Err(error) => self.push_hook_runs(vec![dispatcher::transport_failure_record(
                    EVENT,
                    &plugin.key().to_string(),
                    session,
                    &error,
                )]),
            }
        }
        Ok(())
    }

    pub async fn dispatch_imported_mcp_elicitation(
        &self,
        input: crate::sdk::ImportedMcpElicitationInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
    ) -> Option<crate::sdk::McpElicitationReply> {
        use crate::sdk::McpElicitationReply;

        let (subscription, method, event) = if input.reply.is_some() {
            (
                HookSubscription::IMPORTED_MCP_ELICITATION_RESULT,
                method::HOOK_IMPORTED_MCP_ELICITATION_RESULT,
                "mcp.elicitation.result",
            )
        } else {
            (
                HookSubscription::IMPORTED_MCP_ELICITATION,
                method::HOOK_IMPORTED_MCP_ELICITATION,
                "mcp.elicitation",
            )
        };
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        let mut reply: Option<McpElicitationReply> = None;
        for plugin in self
            .imported_hook_observers(subscription)
            .filter(|plugin| enabled(&plugin.key()))
        {
            let result = self
                .call_session_hook(
                    plugin,
                    method,
                    params.clone(),
                    self.timeouts.fast_or(Duration::from_secs(10)),
                    session,
                )
                .await;
            match result {
                Ok(raw) => match serde_json::from_value::<Option<McpElicitationReply>>(raw) {
                    Ok(Some(next)) => match &mut reply {
                        Some(previous) => previous.prefer(next),
                        None => reply = Some(next),
                    },
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(plugin = %plugin.key(), %error, "invalid MCP elicitation hook result")
                    }
                },
                Err(error) => self.push_hook_runs(vec![dispatcher::transport_failure_record(
                    event,
                    &plugin.key().to_string(),
                    session,
                    &error,
                )]),
            }
        }
        reply
    }

    pub async fn dispatch_imported_skill_shell_before(
        &self,
        input: crate::sdk::ImportedSkillShellHookInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
    ) -> Option<crate::sdk::ImportedSkillShellDecision> {
        use crate::sdk::ImportedSkillShellDecision;
        const EVENT: &str = "skill.shell.before";
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        let mut skip = None;
        for plugin in self
            .imported_hook_observers(HookSubscription::IMPORTED_SKILL_SHELL_BEFORE)
            .filter(|plugin| enabled(&plugin.key()))
        {
            let raw = match self
                .call_session_hook(
                    plugin,
                    method::HOOK_IMPORTED_SKILL_SHELL_BEFORE,
                    params.clone(),
                    self.timeouts.fast_or(Duration::from_secs(10)),
                    session,
                )
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        EVENT,
                        &plugin.key().to_string(),
                        session,
                        &error,
                    )]);
                    continue;
                }
            };
            let decision = match serde_json::from_value::<Option<ImportedSkillShellDecision>>(raw) {
                Ok(Some(decision)) => decision,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(plugin = %plugin.key(), %error, "invalid skill shell hook decision");
                    continue;
                }
            };
            let reason = match &decision {
                ImportedSkillShellDecision::Skip(reason)
                | ImportedSkillShellDecision::Stop(reason) => reason,
            };
            self.push_hook_runs(vec![HookRunRecord::new(
                EVENT,
                plugin.key().to_string(),
                session,
                HookRunStatus::Applied,
                "imported hook blocked a skill shell command",
                Some(reason.clone()),
            )]);
            match decision {
                ImportedSkillShellDecision::Stop(_) => return Some(decision),
                ImportedSkillShellDecision::Skip(_) => {
                    skip.get_or_insert(decision);
                }
            }
        }
        skip
    }

    pub async fn dispatch_imported_model_attempt(
        &self,
        input: crate::sdk::ImportedModelAttemptInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
    ) -> crate::sdk::ImportedModelAttemptCharge {
        use crate::sdk::ImportedModelAttemptCharge;

        let before = input.outcome.is_none();
        let (subscription, method, event) = if before {
            (
                HookSubscription::IMPORTED_MODEL_ATTEMPT_BEFORE,
                method::HOOK_IMPORTED_MODEL_ATTEMPT_BEFORE,
                "model.attempt.before",
            )
        } else {
            (
                HookSubscription::IMPORTED_MODEL_ATTEMPT_AFTER,
                method::HOOK_IMPORTED_MODEL_ATTEMPT_AFTER,
                "model.attempt.after",
            )
        };
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        let mut combined = ImportedModelAttemptCharge::default();
        for plugin in self
            .imported_hook_observers(subscription)
            .filter(|plugin| enabled(&plugin.key()))
        {
            let timeout = plugin
                .agent_plugin()
                .expect("imported hook observer")
                .model_attempt_timeout(before);
            let result = self
                .call_session_hook(
                    plugin,
                    method,
                    params.clone(),
                    self.timeouts.fast_or(timeout),
                    session,
                )
                .await;
            match result {
                Ok(_) if before => {}
                Ok(value) => match serde_json::from_value::<ImportedModelAttemptCharge>(value) {
                    Ok(charge) => {
                        let units = charge.extra_codex_rollout_budget_units;
                        let sum = combined.extra_codex_rollout_budget_units + units;
                        if units >= 0.0 && sum.is_finite() {
                            combined.extra_codex_rollout_budget_units = sum;
                        } else {
                            tracing::warn!(plugin = %plugin.key(), "invalid Codex model request hook charge");
                        }
                    }
                    Err(_) => {
                        tracing::warn!(plugin = %plugin.key(), "invalid Codex model request hook charge")
                    }
                },
                Err(error) => self.push_hook_runs(vec![dispatcher::transport_failure_record(
                    event,
                    &plugin.key().to_string(),
                    session,
                    &error,
                )]),
            }
        }
        combined
    }

    pub async fn dispatch_imported_model_switch(
        &self,
        input: crate::sdk::ImportedModelSwitchInput,
        before: bool,
        enabled: impl Fn(&PluginKey) -> bool + Send,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::ImportedModelSwitchPatch {
        use crate::sdk::{ImportedModelSwitchDecision, ImportedModelSwitchPatch};
        let (subscription, method, event) = if before {
            (
                HookSubscription::IMPORTED_MODEL_SWITCH_BEFORE,
                method::HOOK_IMPORTED_MODEL_SWITCH_BEFORE,
                "model.switch.before",
            )
        } else {
            (
                HookSubscription::IMPORTED_MODEL_SWITCH_AFTER,
                method::HOOK_IMPORTED_MODEL_SWITCH_AFTER,
                "model.switch.after",
            )
        };
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        let mut combined = ImportedModelSwitchPatch::default();
        let mut context_budget = 16 * 1024;
        let mut message_budget = 8 * 1024;
        let mut runs = Vec::new();
        for plugin in self
            .imported_hook_observers(subscription)
            .filter(|plugin| enabled(&plugin.key()))
        {
            let key = plugin.key().to_string();
            let timeout = plugin
                .agent_plugin()
                .expect("imported hook observer")
                .model_switch_timeout(before);
            let result = await_transport_with_cancellation(
                Some(cancellation.clone()),
                self.call_session_hook(plugin, method, params.clone(), timeout, session),
            )
            .await;
            let value = match result {
                Ok(value) => value,
                Err(TransportError::Cancelled) => break,
                Err(error) => {
                    if before && matches!(&error, TransportError::Timeout(_)) {
                        combined.apply_decision(ImportedModelSwitchDecision::Deny(format!(
                            "PreModelSwitch hook from {key} timed out"
                        )));
                    }
                    runs.push(dispatcher::transport_failure_record(
                        event, &key, session, &error,
                    ));
                    continue;
                }
            };
            let patch: ImportedModelSwitchPatch = match serde_json::from_value(value) {
                Ok(patch) => patch,
                Err(error) => {
                    runs.push(HookRunRecord::new(
                        event,
                        &key,
                        session,
                        HookRunStatus::Failed,
                        "invalid model switch hook response",
                        Some(error.to_string()),
                    ));
                    continue;
                }
            };
            if patch.decision.is_none()
                && patch.context.is_empty()
                && patch.system_messages.is_empty()
            {
                continue;
            }
            let detail = if let Some(
                ImportedModelSwitchDecision::Ask(reason)
                | ImportedModelSwitchDecision::Deny(reason),
            ) = &patch.decision
            {
                Some(reason.clone())
            } else {
                patch.system_messages.first().cloned()
            };
            runs.push(HookRunRecord::new(
                event,
                &key,
                session,
                HookRunStatus::Applied,
                "imported model switch hook ran",
                detail,
            ));
            if before {
                if let Some(decision) = patch.decision {
                    combined.apply_decision(decision);
                }
            } else {
                extend_hook_text(&mut combined.context, patch.context, &mut context_budget);
            }
            extend_hook_text(
                &mut combined.system_messages,
                patch.system_messages,
                &mut message_budget,
            );
        }
        self.push_hook_runs(runs);
        combined
    }

    /// Use the first applicable worktree provider in configured plugin order.
    pub async fn dispatch_worktree_hook(
        &self,
        input: crate::sdk::WorktreeHookInput,
        enabled: impl Fn(&PluginKey) -> bool + Send,
    ) -> Result<Option<crate::sdk::WorktreeHookOutput>, PluginError> {
        use crate::sdk::{WorktreeHookOutput, WorktreeOperation};

        let create = matches!(&input.operation, WorktreeOperation::Create { .. });
        let (subscription, method, event) = if create {
            (HookSubscription::WORKTREE_CREATE, method::HOOK_WORKTREE_CREATE, "worktree.create")
        } else {
            (HookSubscription::WORKTREE_REMOVE, method::HOOK_WORKTREE_REMOVE, "worktree.remove")
        };
        let session = Some(input.session_id);
        let params = serde_json::json!(input);
        for plugin in self.plugins.iter().map(Arc::as_ref).filter(|plugin| {
            plugin.subscribes(subscription)
                && !plugin.effect_scope().cancellation_token().is_cancelled()
                && enabled(&plugin.key())
        }) {
            let timeout = plugin.agent_plugin().map_or(Duration::from_secs(30), |package| {
                package.worktree_timeout(create)
            });
            let dispatch = self.call_session_hook(
                plugin, method, params.clone(), self.timeouts.fast_or(timeout), session,
            );
            let result = if create {
                dispatch.await
            } else {
                super::SessionLifecycleScope::WorktreeRemoval(input.session_id).run(dispatch).await
            };
            let output = match result {
                Ok(output) => output,
                Err(error) => {
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        event, &plugin.key().to_string(), session, &error,
                    )]);
                    return Err(PluginError::internal(format!(
                        "{event} from {} failed: {error}", plugin.key(),
                    )));
                }
            };
            let output = serde_json::from_value::<Option<WorktreeHookOutput>>(output)
                .map_err(|error| PluginError::invalid_params_error(&error))?;
            match output {
                None => continue,
                Some(ref result)
                    if create == matches!(result, WorktreeHookOutput::Created { .. }) =>
                {
                    self.push_hook_runs(vec![HookRunRecord::new(
                        event, plugin.key().to_string(), session, HookRunStatus::Applied,
                        "worktree hook ran", None,
                    )]);
                    return Ok(output);
                }
                Some(_) => return Err(PluginError::invalid_params("invalid worktree hook reply")),
            }
        }
        Ok(None)
    }

    // ── session.end ────────────────────────────────────────────────────────

    pub async fn broadcast_shutdown_session_end(&self, input: SessionEndInput) {
        super::SessionLifecycleScope::Shutdown(input.session_id)
            .run(self.broadcast_session_end(input))
            .await;
    }

    pub async fn broadcast_session_end(&self, input: SessionEndInput) {
        if matches!(input.reason, crate::sdk::SessionEndReason::UserExit) {
            super::SessionLifecycleScope::Disposal(input.session_id)
                .run(self.notify_session_end(input))
                .await;
        } else {
            self.notify_session_end(input).await;
        }
    }

    async fn notify_session_end(&self, input: SessionEndInput) {
        let timeout = Duration::from_secs(5);
        let mut notifications = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::SESSION_END) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            let host_handle = Arc::clone(&self._host_handle);
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
                    host_handle.run_in_authorized_callback_context(
                        &plugin.key(),
                        context,
                        plugin.transport.notify(method::HOOK_SESSION_END, params),
                    ),
                )
                .await;
                match notified {
                    Ok(Ok(())) => HookRunRecord::new(
                        "session.end",
                        plugin.key().to_string(),
                        Some(input.session_id),
                        HookRunStatus::Applied,
                        "session.end hook notified",
                        None,
                    ),
                    Ok(Err(error)) => dispatcher::transport_failure_record(
                        "session.end",
                        &plugin.key().to_string(),
                        Some(input.session_id),
                        &error,
                    ),
                    Err(_) => HookRunRecord::new(
                        "session.end",
                        plugin.key().to_string(),
                        Some(input.session_id),
                        HookRunStatus::TimedOut,
                        "session.end hook timed out",
                        None,
                    ),
                }
            });
        }
        let records = futures_util::future::join_all(notifications).await;
        self.push_hook_runs(records);
        let scope = PluginScopeKey::session(input.session_id);
        self._host_handle.dispose_tool_scope(&scope);
        self.command_registry.clear_scope_tree(&scope);
    }

    // ── prompt.submit ─────────────────────────────────────────────────

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
        let session_id = input.session_id;
        let output = self
            .dispatch_prompt_submit_settings(input, cancellation, false, |_| true)
            .await?;
        Ok(UserPromptSubmitInput {
            session_id,
            prompt: output.prompt,
        })
    }

    pub async fn dispatch_user_prompt_submit_with_context(
        &self,
        input: UserPromptSubmitInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
        enabled: impl Fn(&PluginKey) -> bool,
    ) -> Result<super::UserPromptSubmitOutput, PluginError> {
        self.dispatch_prompt_submit_settings(input, cancellation, true, enabled)
            .await
    }

    async fn dispatch_prompt_submit_settings(
        &self,
        input: UserPromptSubmitInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
        separate_context: bool,
        enabled: impl Fn(&PluginKey) -> bool,
    ) -> Result<super::UserPromptSubmitOutput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        let session_id = Some(input.session_id);
        let mut current = input;
        let mut additional_context = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::USER_PROMPT_SUBMIT) || !enabled(&plugin.key()) {
                continue;
            }
            let plugin_id = plugin.key().to_string();
            let params = serde_json::to_value(&current)
                .map_err(|e| PluginError::invalid_params_error(&e))?;
            let v = match await_transport_with_cancellation(
                cancellation.clone(),
                self.call_session_hook(
                    plugin,
                    method::HOOK_USER_PROMPT_SUBMIT,
                    params,
                    timeout,
                    Some(current.session_id),
                ),
            )
            .await
            {
                Ok(v) => v,
                Err(err) => {
                    self.push_hook_runs(vec![dispatcher::transport_failure_record(
                        "prompt.submit",
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
            let patch: Option<UserPromptSubmitPatch> =
                serde_json::from_value(v).map_err(|e| PluginError::invalid_params_error(&e))?;
            if let Some(p) = patch {
                if let Some(r) = p.block_reason {
                    self.push_hook_runs(vec![HookRunRecord::new(
                        "prompt.submit",
                        &plugin_id,
                        session_id,
                        HookRunStatus::Applied,
                        format!("prompt.submit hook blocked prompt: {r}"),
                        Some(r.clone()),
                    )]);
                    let detail = agena_failure::diagnostic::scrubbed_preserve(&r, 220);
                    return Err(PluginError::from_kind_with_public_detail(
                        crate::sdk::PluginErrorKind::PolicyDenied,
                        format_args!("prompt blocked: {r}"),
                        detail,
                    ));
                }
                self.push_hook_runs(vec![HookRunRecord::new(
                    "prompt.submit",
                    &plugin_id,
                    session_id,
                    HookRunStatus::Applied,
                    "prompt.submit hook ran",
                    None,
                )]);
                if let Some(text) = p.prompt {
                    current.prompt = text;
                }
                if let Some(ctx) = p.additional_context {
                    if separate_context {
                        additional_context.push(ctx);
                    } else {
                        current.prompt.push('\n');
                        current.prompt.push_str(&ctx);
                    }
                }
            }
        }
        Ok(super::UserPromptSubmitOutput {
            prompt: current.prompt,
            additional_context,
        })
    }

    // ── tool.failure ───────────────────────────────────────────────

    pub async fn broadcast_tool_failure(
        &self,
        input: ToolFailureInput,
        imported_is_enabled: impl Fn(&PluginKey) -> bool + Send,
    ) {
        let timeout = Duration::from_secs(5);
        let mut notifications = Vec::new();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::TOOL_FAILURE) {
                continue;
            }
            if input.automatic_denial.is_some()
                && plugin.agent_plugin().is_some()
                && !imported_is_enabled(&plugin.key())
            {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            let host = Arc::clone(&self._host_handle);
            notifications.push(async move {
                let plugin_id = plugin.key();
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(error) => {
                        tracing::error!(
                            plugin_id = %plugin_id,
                            hook = method::HOOK_TOOL_FAILURE,
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to serialize plugin tool-failure hook payload",
                                &error,
                            ),
                            "plugin tool-failure hook payload serialization failed"
                        );
                        return;
                    }
                };
                let context = tool_hook_context(
                    &plugin,
                    input.tool_name(),
                    Some(input.session_id),
                    Some(input.call_id),
                    Some(input.workspace_root.clone()),
                );
                let notified = if plugin.agent_plugin().is_some() {
                    let timeout = plugin.agent_plugin()
                        .and_then(|package| package.agent_hook_timeout(method::HOOK_TOOL_FAILURE))
                        .map_or(timeout, |budget| timeout.max(budget + Duration::from_secs(1)));
                    tokio::time::timeout(timeout, host.run_in_authorized_callback_context(
                        &plugin.key(),
                        context,
                        plugin.transport.notify(method::HOOK_TOOL_FAILURE, params),
                    )).await
                } else {
                    tokio::time::timeout(timeout, plugin.transport.notify(method::HOOK_TOOL_FAILURE, params)).await
                };
                match notified
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => tracing::warn!(
                        plugin_id = %plugin_id,
                        hook = method::HOOK_TOOL_FAILURE,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "plugin tool-failure hook delivery failed",
                            &error,
                        ),
                        "best-effort plugin tool-failure notification was not delivered"
                    ),
                    Err(error) => tracing::warn!(
                        plugin_id = %plugin_id,
                        hook = method::HOOK_TOOL_FAILURE,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "plugin tool-failure hook timed out",
                            &error,
                        ),
                        "best-effort plugin tool-failure notification timed out"
                    ),
                }
            });
        }
        futures_util::future::join_all(notifications).await;
    }

    // ── tool.definition ────────────────────────────────────────────────────

    pub async fn dispatch_tool_definition(
        &self,
        input: ToolDefinitionInput,
        session_id: Option<i64>,
        workspace_root: Option<String>,
    ) -> Result<ToolDefinitionInput, PluginError> {
        self.dispatch_tool_definition_plugins(&self.plugins, input, session_id, workspace_root)
            .await
    }

    async fn dispatch_tool_definition_plugins(
        &self,
        plugins: &[Arc<LoadedPlugin>],
        input: ToolDefinitionInput,
        session_id: Option<i64>,
        workspace_root: Option<String>,
    ) -> Result<ToolDefinitionInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let mut runs = Vec::new();
        let result = Box::pin(
            dispatcher::chain_patch_in_context::<ToolDefinitionInput, ToolDefinitionPatch, _, _>(
                plugins,
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
                    if let Some(hidden) = patch.hidden {
                        inp.hidden = hidden;
                    }
                },
                |plugin, input| {
                    Some(tool_hook_context(
                        plugin,
                        input.tool_name(),
                        session_id,
                        None,
                        workspace_root.clone(),
                    ))
                },
                Some(&self._host_handle),
                None,
                &mut runs,
            ),
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
        session_id: Option<i64>,
        workspace_root: Option<String>,
    ) -> Vec<Result<ToolDefinitionInput, PluginError>> {
        self.dispatch_tool_definitions_for_enabled(inputs, session_id, workspace_root, |_| true)
            .await
    }

    pub async fn dispatch_tool_definitions_for_enabled(
        &self,
        inputs: Vec<ToolDefinitionInput>,
        session_id: Option<i64>,
        workspace_root: Option<String>,
        enabled: impl Fn(&PluginKey) -> bool,
    ) -> Vec<Result<ToolDefinitionInput, PluginError>> {
        const CATALOG_DEADLINE: Duration = Duration::from_secs(5);
        let plugins: Vec<Arc<LoadedPlugin>> = self
            .plugins
            .iter()
            .filter(|plugin| {
                plugin.subscribes(HookSubscription::TOOL_DEFINITION) && enabled(&plugin.key())
            })
            .cloned()
            .collect();
        if plugins.is_empty() {
            return inputs.into_iter().map(Ok).collect();
        }
        let input_count = inputs.len();
        let dispatch = async {
            let mut outputs = Vec::with_capacity(input_count);
            for input in inputs {
                outputs.push(
                    self.dispatch_tool_definition_plugins(
                        &plugins,
                        input,
                        session_id,
                        workspace_root.clone(),
                    )
                    .await,
                );
            }
            outputs
        };
        match tokio::time::timeout(CATALOG_DEADLINE, dispatch).await {
            Ok(outputs) => outputs,
            Err(error) => (0..input_count)
                .map(|_| {
                    Err(PluginError::timeout_with_public_detail(
                        agena_failure::diagnostic::format_error_chain_with_context(
                            format!(
                                "tool.definition catalog pass for {input_count} inputs did not complete within {}ms",
                                CATALOG_DEADLINE.as_millis()
                            ),
                            &error,
                        ),
                        "Plugin tool definitions could not be loaded in time. Retry after the plugin finishes starting.",
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
        self.dispatch_agent_stop_hooks(
            input,
            cancellation,
            method::HOOK_AGENT_STOP,
            self.plugins.iter().map(Arc::as_ref),
        ).await
    }

    fn imported_claude_stop_observers(&self) -> impl Iterator<Item = &LoadedPlugin> {
        self.imported_hook_observers(HookSubscription::AGENT_STOP).filter(|plugin| {
            plugin.agent_plugin().is_some_and(|package| package.has_claude_stop_hooks())
        })
    }

    pub fn has_imported_claude_stop_hook(&self, enabled: impl Fn(&PluginKey) -> bool) -> bool {
        self.imported_claude_stop_observers().any(|plugin| enabled(&plugin.key()))
    }

    /// Claude observes a finished response even when its session still owns
    /// a running process or a scheduled wakeup.
    pub async fn dispatch_imported_claude_stop(
        &self,
        input: AgentStopInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
        enabled: impl Fn(&PluginKey) -> bool + Send,
    ) -> Result<AgentStopDispatch, PluginError> {
        self.dispatch_agent_stop_hooks(
            input,
            cancellation,
            method::HOOK_IMPORTED_CLAUDE_STOP,
            self.imported_claude_stop_observers().filter(move |plugin| enabled(&plugin.key())),
        ).await
    }

    async fn dispatch_agent_stop_hooks<'a>(
        &'a self,
        input: AgentStopInput,
        cancellation: Option<tokio_util::sync::CancellationToken>,
        rpc: &'static str,
        subscribers: impl Iterator<Item = &'a LoadedPlugin> + Send,
    ) -> Result<AgentStopDispatch, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(30));
        let mut continue_with_message = None;
        let mut reason = None;
        let mut runs = Vec::new();
        // The notifications plugin observes the final stop decision to update
        // terminal activity. Run it after decision hooks so a continuation
        // cannot briefly project an idle title.
        let mut stop_plugins = subscribers.collect::<Vec<_>>();
        stop_plugins.sort_by_key(|plugin| plugin.key().to_string() == "agena.notifications");
        for plugin in stop_plugins {
            if !plugin.subscribes(HookSubscription::AGENT_STOP) {
                continue;
            }
            let plugin_id = plugin.key().to_string();
            let params =
                serde_json::to_value(&input).map_err(|e| PluginError::invalid_params_error(&e))?;
            let context = HostCallbackContext {
                session_id: Some(input.session_id),
                ..Default::default()
            };
            let v = match await_transport_with_cancellation(
                cancellation.clone(),
                self._host_handle.run_in_authorized_callback_context(
                    &plugin.key(),
                    context,
                    call_with_timeout(plugin, rpc, params, timeout),
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
            let patch: Option<AgentStopPatch> =
                serde_json::from_value(v).map_err(|e| PluginError::invalid_params_error(&e))?;
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
                    self.push_hook_runs(vec![
                        HookRunRecord::new(
                            "agent.stop",
                            &plugin_id,
                            Some(input.session_id),
                            HookRunStatus::Applied,
                            summary,
                            detail,
                        )
                        .with_message(p.continue_with_message.clone()),
                    ]);
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

    // ── agent.cancel ───────────────────────────────────────────────────────

    /// Notify plugins that a user cancellation was accepted for an active
    /// execution. Cancellation itself has already been requested before this
    /// method is called, so hook failures are deliberately best-effort and
    /// never change the cancellation result.
    pub async fn dispatch_agent_cancel(&self, input: AgentCancelInput) {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::AGENT_CANCEL) {
                continue;
            }

            let plugin_id = plugin.key().to_string();
            let params = match serde_json::to_value(&input) {
                Ok(params) => params,
                Err(error) => {
                    tracing::warn!(
                        target: "agena_plugin_host::agent_cancel",
                        plugin = %plugin_id,
                        error = %error,
                        "failed to serialize agent.cancel input"
                    );
                    continue;
                }
            };
            let context = HostCallbackContext {
                session_id: Some(input.session_id),
                ..Default::default()
            };

            if let Err(error) = self
                ._host_handle
                .run_in_authorized_callback_context(
                    &plugin.key(),
                    context,
                    call_with_timeout(plugin, method::HOOK_AGENT_CANCEL, params, timeout),
                )
                .await
            {
                tracing::warn!(
                    target: "agena_plugin_host::agent_cancel",
                    plugin = %plugin_id,
                    session_id = input.session_id,
                    execution_id = %input.execution_id,
                    error = %error,
                    "agent.cancel hook failed; cancellation continues"
                );
            }
        }
    }

    // ── shell.after ──────────────────────────────────────────────

    pub async fn dispatch_command_after(
        &self,
        input: CommandAfterInput,
    ) -> Result<CommandAfterInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        let session_id = input.session_id;
        let mut runs = Vec::new();
        let result =
            dispatcher::chain_patch_in_context::<CommandAfterInput, CommandAfterPatch, _, _>(
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
                |plugin, input| {
                    Some(HostCallbackContext {
                        plugin_id: Some(plugin.key().to_string()),
                        session_id: input.session_id,
                        ..HostCallbackContext::default()
                    })
                },
                Some(&self._host_handle),
                session_id,
                &mut runs,
            )
            .await;
        self.push_hook_runs(runs);
        result.map_err(transport_to_plugin_error)
    }

    // ── chat.messages ────────────────────────────────────────────

    pub async fn dispatch_chat_messages_transform(
        &self,
        input: ChatMessagesTransformInput,
    ) -> Result<ChatMessagesTransformInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(10));
        let session_id = Some(input.session_id);
        let mut runs = Vec::new();
        let result = dispatcher::chain_patch_in_context::<
            ChatMessagesTransformInput,
            ChatMessagesTransformPatch,
            _,
            _,
        >(
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
            |plugin, input| {
                // Recall and other input hooks must query the same session
                // policy and workspace as the pending model request. No tool-call
                // authority is implied by a model-input transformation.
                Some(HostCallbackContext {
                    plugin_id: Some(plugin.key().to_string()),
                    session_id: Some(input.session_id),
                    workspace_root: input.workspace_root.clone().filter(|root| !root.is_empty()),
                    ..Default::default()
                })
            },
            Some(&self._host_handle),
            session_id,
            &mut runs,
        )
        .await;
        self.push_hook_runs(runs);
        result.map_err(transport_to_plugin_error)
    }

    /// Async shutdown — sends `meta/shutdown` and closes every transport.
    /// Plugins whose transport has been transferred to a successor host during
    /// hot-reload keep running; this host still disposes its old registrations.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_report().await;
    }

    /// Send meta/shutdown and dispose every generation-owned effect once.
    /// Concurrent callers share a cached report; cancelling a caller does not
    /// cancel the owned operation or repeat the plugin shutdown callback.
    pub async fn shutdown_report(&self) -> super::PluginShutdownReport {
        use super::{PluginShutdownFailure, PluginShutdownReport};
        use futures_util::FutureExt;

        let mut receiver = {
            let mut cached = self
                .shutdown_result
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some(receiver) = cached.as_ref() {
                receiver.clone()
            } else {
                let (sender, receiver) = tokio::sync::watch::channel(None);
                *cached = Some(receiver.clone());
                let plugins = self.plugins.clone();
                let transferred = self.transferred_to_successor.clone();
                let host = self._host_handle.clone();
                let logs = self.logs.clone();
                let statuses = self.statuses.clone();
                let work = async move {
                    let transferred = transferred.lock().await.clone();
                    let mut report = PluginShutdownReport::default();
                    for plugin in plugins {
                        let plugin_id = plugin.key();
                        let moved = transferred.contains(&plugin_id);
                        if !moved {
                            let close = std::panic::AssertUnwindSafe(shutdown_transport(
                                Arc::clone(&plugin.transport),
                            ))
                            .catch_unwind()
                            .await;
                            let message = match close {
                                Ok(Ok(())) => None,
                                Ok(Err(error)) => Some(
                                    agena_failure::diagnostic::format_error_chain_with_context(
                                        "plugin shutdown did not reach quiescence",
                                        &error,
                                    ),
                                ),
                                Err(_) => Some("plugin transport shutdown panicked".to_owned()),
                            };
                            if let Some(message) = message {
                                logs.append(
                                    &plugin_id,
                                    "error",
                                    "shutdown",
                                    message.clone(),
                                    serde_json::Value::Null,
                                );
                                report.failures.push(PluginShutdownFailure {
                                    plugin_id: plugin_id.to_string(),
                                    stage: "transport".into(),
                                    message,
                                });
                            }
                        }
                        // Read the scope's cached result, not a bounded log ring
                        // which may already have evicted a disposal error.
                        let scope = host.effect_scope(&plugin_id);
                        let disposed = std::panic::AssertUnwindSafe(async {
                            host.dispose_plugin_resources(&plugin_id).await;
                            if let Some(scope) = scope {
                                scope.dispose().await.errors
                            } else {
                                Vec::new()
                            }
                        })
                        .catch_unwind()
                        .await;
                        let errors = match disposed {
                            Ok(errors) => errors,
                            Err(_) => vec!["plugin effect cleanup panicked".to_owned()],
                        };
                        report.failures.extend(errors.into_iter().map(|message| {
                            PluginShutdownFailure {
                                plugin_id: plugin_id.to_string(),
                                stage: "effects".into(),
                                message,
                            }
                        }));
                        if !moved {
                            statuses.record_stopped(&plugin_id);
                        }
                    }
                    report
                };
                match tokio::runtime::Handle::try_current() {
                    Ok(executor) => {
                        executor.spawn(async move {
                            let report = std::panic::AssertUnwindSafe(work)
                                .catch_unwind()
                                .await
                                .unwrap_or_else(|_| PluginShutdownReport {
                                    failures: vec![PluginShutdownFailure {
                                        plugin_id: "<host>".into(),
                                        stage: "shutdown_task".into(),
                                        message: "plugin shutdown task panicked".into(),
                                    }],
                                });
                            if !report.is_success() {
                                tracing::error!(%report, "plugin host shutdown failed");
                            }
                            sender.send_replace(Some(report));
                        });
                    }
                    Err(error) => {
                        sender.send_replace(Some(PluginShutdownReport {
                            failures: vec![PluginShutdownFailure {
                                plugin_id: "<host>".into(),
                                stage: "shutdown_task".into(),
                                message: format!(
                                    "plugin shutdown requires a Tokio executor: {error}"
                                ),
                            }],
                        }));
                    }
                }
                receiver
            }
        };
        loop {
            let report = receiver.borrow_and_update().clone();
            if let Some(report) = report {
                return report;
            }
            if receiver.changed().await.is_err() {
                return PluginShutdownReport {
                    failures: vec![PluginShutdownFailure {
                        plugin_id: "<host>".into(),
                        stage: "shutdown_task".into(),
                        message: "plugin shutdown owner ended without a result".into(),
                    }],
                };
            }
        }
    }

    /// Direct access to the host-side bidirectional router. Used by HTTP
    /// callback routes and bidirectional stdio transports.
    pub fn host_handle(&self) -> Arc<HostHandle> {
        Arc::clone(&self._host_handle)
    }

    pub fn display_contributions(&self) -> Vec<HostDisplayContribution> {
        self.surface_catalog().terminal.display
    }

    pub fn theme_palettes(&self) -> Vec<HostThemePalette> {
        self.surface_catalog().terminal.themes
    }

    /// Plugin-emitted notifications through the unified `host.notify` entry
    /// Bounded recent queue; structured attention events carry stable IDs for
    /// frontend consumption and request resolution.
    pub fn host_notifications(&self) -> Vec<HostNotification> {
        self._host_handle.host_notifications()
    }

    pub fn command_catalog(&self) -> Vec<CommandCatalogItem> {
        self.command_catalog_for_session(None)
    }

    pub fn command_catalog_for_session(&self, session_id: Option<i64>) -> Vec<CommandCatalogItem> {
        let scope = session_id.map(PluginScopeKey::session);
        self.command_catalog_visible(scope.as_ref())
    }

    pub fn command_catalog_for_scope(&self, scope: &PluginScopeKey) -> Vec<CommandCatalogItem> {
        self.command_catalog_visible(Some(scope))
    }

    fn command_catalog_visible(&self, scope: Option<&PluginScopeKey>) -> Vec<CommandCatalogItem> {
        let mut commands = self
            .command_registry
            .visible(scope)
            .into_values()
            .map(|entry| {
                let mut item = entry.value;
                item.submits_prompt = self
                    .plugins_by_id
                    .get(&item.plugin_id)
                    .and_then(|plugin| plugin.agent_plugin())
                    .is_some_and(|package| package.has_inline_prompt_command(&item.command.id));
                item.accepts_attachments =
                    self.command_accepts_attachments(&item.plugin_id, &item.command.id);
                item
            })
            .collect::<Vec<_>>();
        sort_command_catalog(&mut commands);
        commands
    }

    pub fn declare_command_scope(
        &self,
        scope: PluginScopeKey,
        parent: Option<PluginScopeKey>,
    ) -> Result<bool, String> {
        match parent {
            Some(parent) => {
                let changed = self.command_registry.parent(&scope).as_ref() != Some(&parent);
                self.command_registry
                    .set_parent(scope, parent)
                    .map_err(|error| {
                        agena_failure::diagnostic::format_error_chain_with_context(
                            "failed to declare the plugin command scope parent",
                            &error,
                        )
                    })?;
                Ok(changed)
            }
            None => Ok(self.command_registry.clear_parent(&scope).is_some()),
        }
    }

    pub fn remove_command_scope(&self, scope: &PluginScopeKey) -> Result<bool, String> {
        Ok(self.command_registry.clear_parent(scope).is_some())
    }

    pub fn surface_catalog(&self) -> PluginSurfaceCatalog {
        self.surface_catalog_for_session(None)
    }

    pub fn surface_catalog_for_session(&self, session_id: Option<i64>) -> PluginSurfaceCatalog {
        let mut display_by_key = BTreeMap::<(PluginKey, String), HostDisplayContribution>::new();
        // Theme IDs are scoped by plugin in the manifest. Keep the owner in
        // the catalog key so two plugins cannot silently overwrite one
        // another while building the aggregate UI catalog.
        let mut themes_by_key = BTreeMap::<(PluginKey, String), HostThemePalette>::new();
        let commands = self.command_catalog_for_session(session_id);

        for plugin in &self.plugins {
            // Declarative manifest display contributions (Phase 6). Dynamic
            // runtime contributions arrive through the host-handle channel.
            for contribution in &plugin.manifest.surface.display {
                let resolved = HostDisplayContribution {
                    plugin_id: plugin.key(),
                    contribution: contribution.clone(),
                };
                display_by_key.insert(
                    (resolved.plugin_id.clone(), resolved.contribution.id.clone()),
                    resolved,
                );
            }

            for theme in &plugin.manifest.surface.terminal.themes {
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

        PluginSurfaceCatalog {
            commands,
            terminal: PluginTerminalSurfaceCatalog { display, themes },
            web_pages: self.web_page_catalog(),
        }
    }

    pub fn resolve_command(&self, plugin_id: &str, command_id: &str) -> Option<CommandDefinition> {
        self.resolve_command_for_session(None, plugin_id, command_id)
    }

    pub fn resolve_command_for_session(
        &self,
        session_id: Option<i64>,
        plugin_id: &str,
        command_id: &str,
    ) -> Option<CommandDefinition> {
        let plugin_key = parse_plugin_key_for_lookup(plugin_id, "resolve plugin command")?;
        let name = command_registry_name(&plugin_key, command_id);
        let scope = session_id.map(PluginScopeKey::session);
        self.command_registry
            .resolve(scope.as_ref(), &name)
            .map(|entry| entry.value.command)
    }

    pub fn resolve_command_for_scope(
        &self,
        scope: &PluginScopeKey,
        plugin_id: &str,
        command_id: &str,
    ) -> Result<Option<CommandDefinition>, String> {
        let plugin_key: PluginKey = plugin_id
            .parse()
            .map_err(|error| format!("invalid plugin id `{plugin_id}`: {error}"))?;
        let name = command_registry_name(&plugin_key, command_id);
        Ok(self
            .command_registry
            .resolve(Some(scope), &name)
            .map(|entry| entry.value.command))
    }

    pub fn resolve_registered_tool_for_plugin_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        scope: Option<&PluginScopeKey>,
    ) -> Option<RegisteredTool> {
        let plugin_key: PluginKey = match plugin_id.parse() {
            Ok(plugin_key) => plugin_key,
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "parse plugin id while resolving registered tool",
                        &error,
                    ),
                    "registered plugin tool lookup rejected an invalid plugin id"
                );
                return None;
            }
        };
        if let Some(scope) = scope {
            let scoped = self._host_handle.scoped_tool_registry();
            let direct = ToolKey::new(plugin_key.clone(), tool_name.to_owned()).ok();
            let canonical = tool_name
                .parse::<ToolKey>()
                .ok()
                .filter(|key| key.plugin() == &plugin_key);
            if let Some(entry) = direct
                .iter()
                .chain(canonical.iter())
                .find_map(|key| scoped.resolve(Some(scope), key))
            {
                return Some(entry.value);
            }
        }
        let registry = match self.tool_registry.read() {
            Ok(registry) => registry,
            Err(error) => {
                tracing::error!(
                    operation = "resolve registered plugin tool",
                    error = %error,
                    "recovering poisoned plugin tool-registry lock"
                );
                error.into_inner()
            }
        };
        registry
            .lookup_for_plugin(&plugin_key, tool_name)
            .cloned()
            .or_else(|| {
                let tool_key: ToolKey = match tool_name.parse() {
                    Ok(tool_key) => tool_key,
                    Err(error) => {
                        tracing::debug!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "parse canonical tool name during registered-tool fallback",
                                &error,
                            ),
                            "registered plugin tool fallback skipped a non-canonical tool name"
                        );
                        return None;
                    }
                };
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
        let mut pending = match self.hook_runs.lock() {
            Ok(pending) => pending,
            Err(error) => {
                tracing::error!(
                    diagnostic = %error,
                    "plugin hook-run queue lock was poisoned; recovering queued hook diagnostics"
                );
                error.into_inner()
            }
        };
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
    AgentCancelInput, AgentStopDispatch, AgentStopHookRun, AgentStopInput, AgentStopPatch, Arc,
    BTreeMap, ChatHeadersInput, ChatHeadersPatch, ChatMessageInput, ChatMessagePatch,
    ChatMessagesTransformInput, ChatMessagesTransformPatch, ChatParamsInput, ChatParamsPatch,
    ChatSystemTransformInput, ChatSystemTransformPatch, CommandAfterInput, CommandAfterPatch,
    CommandBeforeInput, CommandBeforeOutcome, CommandBeforeResponse, CommandCatalogItem,
    CommandDefinition, CommandDispatch, CommandInvokeInput, CommandResult, ConfigInput,
    ConfigPatch, Duration, HashMap, HookRunRecord, HookRunStatus, HookSubscription,
    HostCallbackContext, HostDisplayContribution, HostHandle, HostNotification, HostThemePalette,
    LoadedPlugin, NoopHostClient, NotificationInput, PluginActivationDiagnostic,
    PluginActivationInspect, PluginArchitectureCatalog, PluginArchitectureEffect,
    PluginArchitectureNode, PluginArchitecturePipeline, PluginDependencyEdge, PluginDependencyKind,
    PluginError, PluginHost, PluginInspect, PluginKey, PluginLogRecord, PluginLogStore,
    PluginScopeKey, PluginServiceBindingKey, PluginServiceImportInspect, PluginServiceInspect,
    PluginSurfaceCatalog, PluginTerminalSurfaceCatalog, PluginToolRegistry, PostRunInput,
    PreRunInput, ProviderListInput, ProviderListPatch, RegisteredTool, RwLock, SessionEndInput,
    SessionStartInput, SessionStartPatch, ShellEnvInput, ShellEnvPatch, TimeoutsConfig,
    ToolAfterDispatch, ToolAfterInput, ToolBeforeBail, ToolBeforeDispatch, ToolBeforeInput,
    ToolDefinitionInput, ToolDefinitionPatch, ToolFailureInput, ToolInvokeInput, ToolInvokeOutput,
    ToolInvokeStream, ToolKey, ToolRegistryChangedEvent, ToolStreamChunk, ToolStreamEnd,
    TransportError, UserPromptSubmitInput, UserPromptSubmitPatch, call_with_timeout,
    command_registry_name, dispatcher, hook_registration_for_plugin, host_api, merge_json, method,
    push_hook_runs_into, shutdown_transport, sort_command_catalog, tool_hook_context,
    transport_to_plugin_error,
};
