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
        let operation_registry = host_handle.operation_registry();
        Arc::new(Self {
            plugins: Vec::new(),
            plugins_by_id: HashMap::new(),
            tool_registry,
            operation_registry,
            operation_pipeline: Arc::new(crate::event_pipeline::PluginAroundPipeline::new()),
            tool_before_pipeline: Arc::new(
                crate::event_pipeline::PluginTransformBailPipeline::new(
                    crate::event_pipeline::PluginPipelineFailurePolicy::Abort,
                ),
            ),
            tool_after_pipeline: Arc::new(crate::event_pipeline::PluginTransformPipeline::new(
                crate::event_pipeline::PluginPipelineFailurePolicy::Abort,
            )),
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
        let scope = host_api::current_host_callback_context()
            .and_then(|context| context.session_id)
            .map(PluginScopeKey::session);
        self.lookup_tool_for_scope(canonical_name, scope.as_ref())
    }

    pub fn lookup_tool_for_scope(
        &self,
        canonical_name: &str,
        scope: Option<&PluginScopeKey>,
    ) -> Option<RegisteredTool> {
        let key: ToolKey = canonical_name.parse().ok()?;
        if let Some(value) = self
            ._host_handle
            .scoped_tool_registry()
            .resolve(scope, &key)
        {
            return Some(value.value);
        }
        self.tool_registry
            .read()
            .ok()?
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
        let mut tools = self
            .tool_registry
            .read()
            .map(|registry| {
                registry
                    .registered_tools()
                    .map(|tool| (tool.tool_key().clone(), tool.clone()))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
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

            dependencies.extend(configured.activation.requires.iter().cloned().map(
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
                            provider_id: binding.provider.parse().ok()?,
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
        let pipelines = vec![
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
                    id: "operation.invoke".to_string(),
                    mode: crate::event_pipeline::PluginEventMode::Around,
                    durable: false,
                    scoped: true,
                },
                failure_policy: None,
                handlers: self.operation_pipeline.inventory(),
            },
        ];
        PluginArchitectureCatalog {
            profiles: self.profile_resolution.clone(),
            reload: self.reload_plan.clone(),
            plugins,
            dependencies,
            effects,
            pipelines,
            tool_registrations: self._host_handle.scoped_tool_registry().inspect(),
            operation_registrations: self.operation_registry.inspect(),
        }
    }

    pub fn plugin_inspect(&self, plugin_id: &str) -> Option<PluginInspect> {
        let status = self.plugin_status(plugin_id)?;
        let plugin_key: PluginKey = plugin_id.parse().ok()?;
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
            .map(|plugin| vec![hook_registration_for_plugin(plugin)])
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
                requires: configured.activation.requires.clone(),
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
            .map_err(|error| PluginError::internal(error.to_string()))?;
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
            .map_err(|error| PluginError::internal(error.to_string()))?;
        Ok(report.value.input)
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
        let tool_name = registered_tool.tool_name().to_string();
        let params =
            serde_json::to_value(&input).map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let plugin_key = plugin.key();
        let invoke = self._host_handle.run_in_authorized_callback_context(
            &plugin_key,
            HostCallbackContext {
                session_id: Some(session_id),
                call_id: Some(call_id),
                workspace_root: Some(workspace_root),
                tool_name: Some(tool_name),
                ..Default::default()
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

    pub fn register_operation_middleware<F, Fut>(
        &self,
        owner_plugin_id: &str,
        priority: i32,
        label: impl Into<String>,
        handler: F,
    ) -> Result<crate::event_pipeline::PluginPipelineRegistration, PluginError>
    where
        F: Fn(
                PluginOperationDispatch,
                crate::event_pipeline::PluginAroundNext<
                    PluginOperationDispatch,
                    PluginOperationResult,
                    PluginError,
                >,
            ) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = Result<PluginOperationResult, PluginError>>
            + Send
            + 'static,
    {
        let owner: PluginKey = owner_plugin_id.parse().map_err(|error| {
            PluginError::invalid_params(format!("invalid middleware owner plugin id: {error}"))
        })?;
        let scope = self._host_handle.effect_scope(&owner).ok_or_else(|| {
            PluginError::not_implemented(format!(
                "plugin `{owner_plugin_id}` has no active effect scope"
            ))
        })?;
        self.operation_pipeline
            .register(&scope, priority, label, handler)
            .map_err(|error| PluginError::internal(error.to_string()))
    }

    pub async fn invoke_plugin_operation_async(
        &self,
        plugin_id: &str,
        mut input: PluginOperationInvokeInput,
    ) -> Result<PluginOperationResult, PluginError> {
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
        if input.operation_id.is_empty() {
            return Err(PluginError::invalid_params(
                "operation_id must not be empty".to_string(),
            ));
        }
        let definition = self
            .resolve_operation(plugin_id, input.operation_id.as_str())
            .ok_or_else(|| {
                PluginError::not_implemented(format!(
                    "plugin `{plugin_id}` does not declare operation `{}`",
                    input.operation_id
                ))
            })?;
        if !matches!(
            &definition.target,
            crate::sdk::PluginOperationTarget::Method { .. }
        ) {
            return Err(PluginError::invalid_params(format!(
                "operation `{}` is tool-backed and must execute through the Runtime operation resolver",
                input.operation_id
            )));
        }
        let input_is_empty = input.input.is_null()
            || input
                .input
                .as_object()
                .is_some_and(serde_json::Map::is_empty);
        input.input = if input_is_empty && !input.raw.trim().is_empty() {
            definition.input.parse_shorthand(input.raw.as_str())
        } else if input_is_empty {
            definition.input.default_value()
        } else {
            definition
                .input
                .validate_value(&input.input)
                .map(|()| input.input.clone())
        }
        .map_err(|error| PluginError::invalid_params(error.to_string()))?;

        let timeout = self.timeouts.fast_or(Duration::from_secs(10));
        let contract = definition.input.clone();
        let dispatch = PluginOperationDispatch::new(plugin_key.clone(), input);
        let host_handle = Arc::clone(&self._host_handle);
        let result = self
            .operation_pipeline
            .dispatch(dispatch, move |dispatch| {
                let plugin = Arc::clone(&plugin);
                let contract = contract.clone();
                let host_handle = Arc::clone(&host_handle);
                async move {
                    let plugin_key = dispatch.plugin_id().clone();
                    let operation_id = dispatch.operation_id().to_string();
                    let input = dispatch.into_input();
                    contract
                        .validate_value(&input.input)
                        .map_err(|error| PluginError::invalid_params(error.to_string()))?;
                    let session_id = input.session_id;
                    let call_id = input.call_id;
                    let workspace_root = input.workspace_root.clone();
                    let params = serde_json::to_value(&input)
                        .map_err(|error| PluginError::invalid_params(error.to_string()))?;
                    let value = host_handle
                        .run_in_authorized_callback_context(
                            &plugin_key,
                            HostCallbackContext {
                                session_id,
                                call_id,
                                workspace_root,
                                tool_name: None,
                                ..Default::default()
                            },
                            call_with_timeout(&plugin, method::OPERATION_INVOKE, params, timeout),
                        )
                        .await
                        .map_err(transport_to_plugin_error)?;
                    let result: PluginOperationResult =
                        serde_json::from_value(value).map_err(|error| {
                            PluginError::invalid_params(format!(
                                "invalid operation result for `{operation_id}`: {error}"
                            ))
                        })?;
                    result.validate().map_err(|error| {
                        PluginError::invalid_params(format!(
                            "invalid operation result for `{operation_id}`: {error}"
                        ))
                    })?;
                    Ok(result)
                }
            })
            .await?;
        result.validate().map_err(|error| {
            PluginError::invalid_params(format!(
                "operation middleware produced an invalid result for `{}`: {error}",
                definition.id
            ))
        })?;
        Ok(result)
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
        let tool_name = registered_tool.tool_name().to_string();
        let workspace_root = input.workspace_root.clone();
        let result = await_transport_with_cancellation(
            cancellation,
            self._host_handle.run_in_authorized_callback_context(
                &plugin.key(),
                HostCallbackContext {
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
        let tool_name = registered_tool.tool_name().to_string();
        let workspace_root = input.workspace_root.clone();
        let result = await_transport_with_cancellation(
            cancellation,
            self._host_handle.run_in_authorized_callback_context(
                &plugin.key(),
                HostCallbackContext {
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
        let (context, callback_authority) = self
            ._host_handle
            .issue_callback_authority(&plugin.key(), context);
        if let Some(stream) = host_api::run_in_host_callback_context(
            context.clone(),
            plugin.transport.invoke_stream(input.clone()),
        )
        .await
        .map_err(transport_to_plugin_error)?
        {
            let stream_id = stream.stream_id;
            let chunks = stream.chunks;
            let native_end = stream.end;
            let (end_tx, end_rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                // Keep callback authority until the transport reports a
                // terminal stream result; chunk consumers may stop earlier.
                let _callback_authority = callback_authority;
                let result = native_end.await.unwrap_or_else(|error| {
                    Err(PluginError::internal(format!(
                        "plugin stream ended without a terminal result: {error}"
                    )))
                });
                let _ = end_tx.send(result);
            });
            return Ok(ToolInvokeStream {
                stream_id,
                chunks,
                end: end_rx,
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
            Some(&self._host_handle),
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
        let scope = PluginScopeKey::session(input.session_id);
        self._host_handle.dispose_tool_scope(&scope);
        self.operation_registry.clear_scope_tree(&scope);
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
                Some(&self._host_handle),
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
                session_id: Some(input.session_id),
                ..Default::default()
            };
            let v = match await_transport_with_cancellation(
                cancellation.clone(),
                self._host_handle.run_in_authorized_callback_context(
                    &plugin.key(),
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

    // ── command.execute.after ──────────────────────────────────────────────

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
            let plugin_id = plugin.key();
            if let Err(error) = shutdown_transport(Arc::clone(&plugin.transport)).await {
                self.logs.append(
                    &plugin_id,
                    "error",
                    "shutdown",
                    format!("plugin shutdown did not reach quiescence: {error}"),
                    serde_json::Value::Null,
                );
            }
            self._host_handle.dispose_plugin_resources(&plugin_id).await;
            self.statuses.record_stopped(&plugin_id);
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
    /// (Phase 6). Bounded recent queue; frontends dedupe/consume by
    /// `plugin_id:severity:body`.
    pub fn host_notifications(&self) -> Vec<HostNotification> {
        self._host_handle.host_notifications()
    }

    pub fn operation_catalog(&self) -> Vec<PluginOperationCatalogItem> {
        let mut operations = self
            .operation_registry
            .visible(None)
            .into_values()
            .map(|entry| entry.value)
            .collect::<Vec<_>>();
        sort_operation_catalog(&mut operations);
        operations
    }

    pub fn operation_catalog_for_scope(
        &self,
        scope: &PluginScopeKey,
    ) -> Vec<PluginOperationCatalogItem> {
        let mut operations = self
            .operation_registry
            .visible(Some(scope))
            .into_values()
            .map(|entry| entry.value)
            .collect::<Vec<_>>();
        sort_operation_catalog(&mut operations);
        operations
    }

    pub fn declare_operation_scope(
        &self,
        scope: PluginScopeKey,
        parent: Option<PluginScopeKey>,
    ) -> Result<bool, String> {
        match parent {
            Some(parent) => {
                let changed = self.operation_registry.parent(&scope).as_ref() != Some(&parent);
                self.operation_registry
                    .set_parent(scope, parent)
                    .map_err(|error| error.to_string())?;
                Ok(changed)
            }
            None => Ok(self.operation_registry.clear_parent(&scope).is_some()),
        }
    }

    pub fn remove_operation_scope(&self, scope: &PluginScopeKey) -> Result<bool, String> {
        Ok(self.operation_registry.clear_parent(scope).is_some())
    }

    pub fn surface_catalog(&self) -> PluginSurfaceCatalog {
        let mut display_by_key = BTreeMap::<(PluginKey, String), HostDisplayContribution>::new();
        // Theme IDs are scoped by plugin in the manifest. Keep the owner in
        // the catalog key so two plugins cannot silently overwrite one
        // another while building the aggregate UI catalog.
        let mut themes_by_key = BTreeMap::<(PluginKey, String), HostThemePalette>::new();
        let operations = self.operation_catalog();

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
            operations,
            terminal: PluginTerminalSurfaceCatalog { display, themes },
        }
    }

    pub fn resolve_operation(
        &self,
        plugin_id: &str,
        operation_id: &str,
    ) -> Option<PluginOperationDefinition> {
        let plugin_key: PluginKey = plugin_id.parse().ok()?;
        let name = operation_registry_name(&plugin_key, operation_id);
        self.operation_registry
            .resolve(None, &name)
            .map(|entry| entry.value.operation)
    }

    pub fn resolve_operation_for_scope(
        &self,
        scope: &PluginScopeKey,
        plugin_id: &str,
        operation_id: &str,
    ) -> Result<Option<PluginOperationDefinition>, String> {
        let plugin_key: PluginKey = plugin_id
            .parse()
            .map_err(|error| format!("invalid plugin id `{plugin_id}`: {error}"))?;
        let name = operation_registry_name(&plugin_key, operation_id);
        Ok(self
            .operation_registry
            .resolve(Some(scope), &name)
            .map(|entry| entry.value.operation))
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
    AgentCancelInput, AgentStopDispatch, AgentStopHookRun, AgentStopInput, AgentStopPatch, Arc,
    AuthInput, AuthOutput, BTreeMap, ChatHeadersInput, ChatHeadersPatch, ChatMessageInput,
    ChatMessagePatch, ChatMessagesTransformInput, ChatMessagesTransformPatch, ChatParamsInput,
    ChatParamsPatch, ChatSystemTransformInput, ChatSystemTransformPatch, CommandAfterInput,
    CommandAfterPatch, CommandBeforeInput, CommandBeforeOutcome, CommandBeforeResponse,
    ConfigInput, ConfigPatch, Duration, EventEnvelope, HashMap, HookRunRecord, HookRunStatus,
    HookSubscription, HostCallbackContext, HostDisplayContribution, HostHandle, HostNotification,
    HostThemePalette, LoadedPlugin, NoopHostClient, NotificationInput, PluginActivationDiagnostic,
    PluginActivationInspect, PluginArchitectureCatalog, PluginArchitectureEffect,
    PluginArchitectureNode, PluginArchitecturePipeline, PluginDependencyEdge, PluginDependencyKind,
    PluginError, PluginHost, PluginInspect, PluginKey, PluginLogRecord, PluginLogStore,
    PluginOperationCatalogItem, PluginOperationDefinition, PluginOperationDispatch,
    PluginOperationInvokeInput, PluginOperationResult, PluginScopeKey, PluginServiceBindingKey,
    PluginServiceImportInspect, PluginServiceInspect, PluginSurfaceCatalog,
    PluginTerminalSurfaceCatalog, PluginToolRegistry, PostRunInput, PreRunInput, ProviderListInput,
    ProviderListPatch, RegisteredTool, RwLock, SessionEndInput, SessionStartInput,
    SessionStartPatch, ShellEnvInput, ShellEnvPatch, TimeoutsConfig, ToolAfterDispatch,
    ToolAfterInput, ToolBeforeBail, ToolBeforeDispatch, ToolBeforeInput, ToolDefinitionInput,
    ToolDefinitionPatch, ToolFailureInput, ToolInvokeInput, ToolInvokeOutput, ToolInvokeStream,
    ToolKey, ToolPermissionNetworksInput, ToolPermissionPathsInput, ToolRegistryChangedEvent,
    ToolStreamChunk, ToolStreamEnd, TransportError, UserPromptSubmitInput, UserPromptSubmitPatch,
    call_with_timeout, dispatcher, hook_registration_for_plugin, host_api, merge_json, method,
    operation_registry_name, push_hook_runs_into, shutdown_transport, sort_operation_catalog,
    tool_hook_context, transport_to_plugin_error,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginManifest;

    #[tokio::test]
    async fn scoped_operation_catalog_uses_the_same_nearest_visibility_resolver_as_lookup() {
        use crate::PluginScopeKey;
        use crate::sdk::{
            OperationDiscoverability, PluginOperationDefinition, PluginOperationTarget,
            SettingsContract, SettingsNode,
        };

        fn item(plugin_id: &PluginKey, title: &str) -> PluginOperationCatalogItem {
            PluginOperationCatalogItem {
                plugin_id: plugin_id.clone(),
                accepts_empty_input: true,
                default_input: serde_json::json!({}),
                operation: PluginOperationDefinition {
                    id: "open".to_string(),
                    title: title.to_string(),
                    description: String::new(),
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
                },
            }
        }

        let host = PluginHost::new_empty();
        let plugin_id: PluginKey = "example.operations".parse().expect("plugin key");
        let workspace: PluginScopeKey = "workspace.main".parse().expect("workspace scope");
        let session: PluginScopeKey = "session.7".parse().expect("session scope");
        host.declare_operation_scope(session.clone(), Some(workspace.clone()))
            .expect("declare session");

        let registry_name = operation_registry_name(&plugin_id, "open");
        let owner = crate::effect_scope::PluginEffectScope::new(plugin_id.clone());
        let _global_registration = host
            .operation_registry
            .register(
                &owner,
                None,
                registry_name.clone(),
                item(&plugin_id, "Global Open"),
                "global operation",
            )
            .expect("global operation");
        let scoped_registration = host
            .operation_registry
            .register(
                &owner,
                Some(session.clone()),
                registry_name.clone(),
                item(&plugin_id, "Session Open"),
                "session operation",
            )
            .expect("session operation");

        assert_eq!(host.operation_catalog()[0].operation.title, "Global Open");
        assert_eq!(
            host.operation_catalog_for_scope(&session)[0]
                .operation
                .title,
            "Session Open"
        );
        assert_eq!(
            host.resolve_operation_for_scope(&session, "example.operations", "open")
                .expect("session lookup")
                .expect("operation")
                .title,
            "Session Open"
        );

        scoped_registration
            .dispose()
            .await
            .expect("dispose session operation");
        assert_eq!(
            host.resolve_operation_for_scope(&session, "example.operations", "open")
                .expect("fallback lookup")
                .expect("operation")
                .title,
            "Global Open"
        );
        host.remove_operation_scope(&session)
            .expect("remove session");
        host.remove_operation_scope(&workspace)
            .expect("remove workspace");
    }

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
        let mut manifest = PluginManifest::new("test", "hanging", "0.1.0");
        manifest.hooks = HookSubscription::TOOL_DEFINITION;
        let host_handle = Arc::new(HostHandle::new_with_components(
            Arc::new(NoopHostClient),
            Arc::clone(&tool_registry),
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(RwLock::new(HashMap::new())),
            Arc::clone(&statuses),
            Arc::clone(&logs),
            None,
        ));
        let operation_registry = host_handle.operation_registry();
        let host = Arc::new(PluginHost {
            plugins: vec![Arc::new(LoadedPlugin::new(
                "static",
                crate::config::ConfiguredPlugin {
                    settings: serde_json::json!({}),
                    ..crate::config::ConfiguredPlugin::default()
                },
                Arc::new(HangingTransport),
                manifest,
                "test".to_string(),
                Vec::new(),
            ))],
            plugins_by_id: Default::default(),
            tool_registry: Arc::clone(&tool_registry),
            operation_registry,
            operation_pipeline: Arc::new(crate::event_pipeline::PluginAroundPipeline::new()),
            tool_before_pipeline: Arc::new(
                crate::event_pipeline::PluginTransformBailPipeline::new(
                    crate::event_pipeline::PluginPipelineFailurePolicy::Abort,
                ),
            ),
            tool_after_pipeline: Arc::new(crate::event_pipeline::PluginTransformPipeline::new(
                crate::event_pipeline::PluginPipelineFailurePolicy::Abort,
            )),
            statuses: Arc::clone(&statuses),
            logs: Arc::clone(&logs),
            configured_plugins: BTreeMap::new(),
            activation_blocks: BTreeMap::new(),
            activation_epochs: BTreeMap::new(),
            reload_plan: Default::default(),
            profile_resolution: Default::default(),
            prefetched_manifests: BTreeMap::new(),
            service_bindings: BTreeMap::new(),
            timeouts: TimeoutsConfig {
                fast: Some(crate::config::DurationSpec(Duration::from_millis(20))),
                ..Default::default()
            },
            _host_handle: host_handle,
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
