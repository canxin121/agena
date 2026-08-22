enum BuildCandidate {
    Prepared(Box<PreparedPlugin>),
    Reused(Arc<LoadedPlugin>),
}

impl BuildCandidate {
    fn manifest(&self) -> &crate::sdk::PluginManifest {
        match self {
            Self::Prepared(plugin) => &plugin.manifest,
            Self::Reused(plugin) => &plugin.manifest,
        }
    }
}

impl PluginHost {
    pub async fn new(build: PluginHostBuildConfig) -> Result<Arc<PluginHost>, HostError> {
        let PluginHostBuildConfig {
            static_plugins,
            config,
            workspace_root,
            agena_version,
            callback_base_url,
            host_client,
            previous,
            previous_plugins,
        } = build;
        let profile_resolution = config.profile_resolution.clone();
        let configured_plugins_for_inspect = config.list.clone();
        let previous_configured = previous_plugins
            .iter()
            .map(|(plugin_id, configured)| (plugin_id.clone(), configured.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut reload_plan =
            crate::activation::plan_plugin_reload(&previous_configured, &config.list)
                .map_err(HostError::Config)?;
        let reusable_plugin_ids = reload_plan.reusable_plugin_ids();
        let initial_plan =
            crate::activation::plan_plugin_activation(&config.list).map_err(HostError::Config)?;
        let mut activation_blocks = initial_plan.blocked.clone();
        let host_inner = host_client.unwrap_or_else(|| Arc::new(NoopHostClient));
        let tool_registry_shared = Arc::new(RwLock::new(PluginToolRegistry::new()));
        let plugin_indices: Arc<RwLock<HashMap<PluginKey, usize>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let plugin_names: Arc<RwLock<HashMap<PluginKey, String>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let statuses_shared = Arc::new(crate::status::StatusRegistry::new());
        let logs_shared = previous
            .as_ref()
            .map(|previous| previous.log_store())
            .unwrap_or_else(|| Arc::new(PluginLogStore::default()));
        let mut handle = HostHandle::new_with_components(
            host_inner,
            Arc::clone(&tool_registry_shared),
            Arc::clone(&plugin_indices),
            Arc::clone(&plugin_names),
            Arc::clone(&statuses_shared),
            Arc::clone(&logs_shared),
            callback_base_url,
        );
        let quotas = Arc::new(crate::quota::QuotaRegistry::new(
            config.host.default_quota.clone(),
        ));
        for (plugin_id, quota) in &config.host.quotas {
            quotas.set_plugin(plugin_id.clone(), quota.clone());
        }
        handle.install_quota_registry(Arc::clone(&quotas));
        let host_handle = Arc::new(handle);
        #[allow(clippy::type_complexity)]
        let env_lookup: Box<dyn Fn(&str) -> Option<String> + Send + Sync> =
            Box::new(|key: &str| std::env::var(key).ok());
        let mut static_registry: HashMap<PluginKey, StaticRegistration> = static_plugins
            .into_iter()
            .map(|entry| (entry.key, entry.registration))
            .collect();

        for (id, configured) in &config.list {
            let plugin_key: PluginKey = id.parse().map_err(|error| {
                HostError::Config(format!("invalid plugin id `{id}` in plugins.list: {error}"))
            })?;
            statuses_shared.set(crate::status::PluginStatus::initial(
                &plugin_key,
                configured.kind_str(),
            ));
            if configured.disabled() {
                statuses_shared.record_stopped(&plugin_key);
            }
        }
        for block in activation_blocks.values() {
            record_activation_block(&statuses_shared, &logs_shared, block);
        }

        let previous_loaded: HashMap<PluginKey, Arc<LoadedPlugin>> = previous
            .as_ref()
            .map(|host| {
                host.plugins
                    .iter()
                    .map(|plugin| (plugin.key(), Arc::clone(plugin)))
                    .collect()
            })
            .unwrap_or_default();
        let mut candidates = BTreeMap::<String, BuildCandidate>::new();
        let mut prefetched_manifests = BTreeMap::new();

        // Prepare every config-viable candidate before any meta/init. Reuse
        // candidates contribute their immutable previous manifest to the graph;
        // final reuse eligibility is checked after service bindings resolve.
        for id in &initial_plan.ordered {
            let configured = config
                .list
                .get(id)
                .expect("activation plan references configured plugin");
            let plugin_key: PluginKey = id.parse().expect("activation planner validated ids");
            let reusable = reusable_plugin_ids.contains(id)
                && !matches!(&configured.package, PluginPackage::Static { .. });
            if reusable && let Some(previous_plugin) = previous_loaded.get(&plugin_key).cloned() {
                prefetched_manifests.insert(id.clone(), previous_plugin.manifest.clone());
                candidates.insert(id.clone(), BuildCandidate::Reused(previous_plugin));
                continue;
            }
            match prepare_entry(
                id,
                configured,
                &mut static_registry,
                Arc::clone(&host_handle),
                &workspace_root,
                &env_lookup,
                &config.host.trusted_keys,
            )
            .await
            {
                Ok(prepared) => {
                    prefetched_manifests.insert(id.clone(), prepared.manifest.clone());
                    candidates.insert(id.clone(), BuildCandidate::Prepared(Box::new(prepared)));
                }
                Err(error) => {
                    let block = crate::activation::PluginActivationBlock {
                        plugin_id: id.clone(),
                        code: "preparation_failed",
                        message: "plugin transport or manifest preparation failed".to_string(),
                        dependencies: Vec::new(),
                    };
                    activation_blocks.insert(id.clone(), block.clone());
                    record_activation_block(&statuses_shared, &logs_shared, &block);
                    let plugin_key: PluginKey = id.parse().expect("valid plugin id");
                    logs_shared.append(
                        &plugin_key,
                        "error",
                        "prepare",
                        error.to_string(),
                        serde_json::Value::Null,
                    );
                    host_handle.rollback_failed_plugin(&plugin_key).await;
                }
            }
        }

        let service_plan = crate::services::resolve_plugin_services(&prefetched_manifests);
        for service_block in service_plan.blocked.values() {
            let block = crate::activation::PluginActivationBlock {
                plugin_id: service_block.plugin_id.clone(),
                code: service_block.code,
                message: service_block.message.clone(),
                dependencies: service_block.dependencies.clone(),
            };
            if activation_blocks
                .insert(service_block.plugin_id.clone(), block.clone())
                .is_none()
            {
                record_activation_block(&statuses_shared, &logs_shared, &block);
            }
        }

        let mut effective_config = config.list.clone();
        for blocked in activation_blocks.keys() {
            if let Some(configured) = effective_config.get_mut(blocked) {
                configured.enabled = false;
            }
        }
        for (consumer, providers) in &service_plan.activation_dependencies {
            let Some(configured) = effective_config.get_mut(consumer) else {
                continue;
            };
            for provider in providers {
                let provider_key: PluginKey = provider.parse().map_err(|error| {
                    HostError::Config(format!(
                        "resolved service provider `{provider}` is not a valid plugin id: {error}"
                    ))
                })?;
                configured
                    .activation
                    .after
                    .retain(|dependency| dependency != &provider_key);
                if !configured.activation.requires.contains(&provider_key) {
                    configured.activation.requires.push(provider_key);
                }
            }
        }
        let effective_plan = crate::activation::plan_plugin_activation(&effective_config)
            .map_err(HostError::Config)?;
        let activation_epochs =
            crate::activation::plugin_activation_epochs(&effective_config, &effective_plan)
                .map_err(HostError::Config)?;
        for block in effective_plan.blocked.values() {
            if activation_blocks
                .insert(block.plugin_id.clone(), block.clone())
                .is_none()
            {
                record_activation_block(&statuses_shared, &logs_shared, block);
            }
        }
        host_handle
            .install_service_bindings(service_plan.bindings.clone())
            .await;

        let previous_service_bindings = previous
            .as_ref()
            .map(|host| host.service_bindings.clone())
            .unwrap_or_default();
        for decision in &mut reload_plan.decisions {
            let plugin_id = decision.plugin_id.to_string();
            let changed_service_providers = service_epoch_changed_providers(
                plugin_id.as_str(),
                &service_plan.bindings,
                &previous_service_bindings,
                &config.list,
                &previous_plugins,
            );
            if !changed_service_providers.is_empty() {
                if decision.action == crate::activation::PluginReloadAction::Reuse {
                    decision.action = crate::activation::PluginReloadAction::Restart;
                }
                if !decision
                    .reasons
                    .contains(&crate::activation::PluginReloadReason::ServiceBindingChanged)
                {
                    decision
                        .reasons
                        .push(crate::activation::PluginReloadReason::ServiceBindingChanged);
                    decision.reasons.sort();
                }
                decision.triggered_by.extend(
                    changed_service_providers
                        .into_iter()
                        .filter_map(|provider| provider.parse().ok()),
                );
                decision.triggered_by.sort();
                decision.triggered_by.dedup();
            }
        }
        let mut loaded = Vec::<Arc<LoadedPlugin>>::new();
        let mut by_id = HashMap::<PluginKey, Arc<LoadedPlugin>>::new();
        let mut activated = BTreeSet::<String>::new();

        for id in effective_plan.ordered {
            let plugin_key: PluginKey = id.parse().expect("activation planner validated id");
            let effective = effective_config
                .get(&id)
                .expect("effective activation entry exists");
            let failed_requirements = effective
                .activation
                .requires
                .iter()
                .map(ToString::to_string)
                .filter(|dependency| !activated.contains(dependency))
                .collect::<Vec<_>>();
            if !failed_requirements.is_empty() {
                let block = crate::activation::PluginActivationBlock {
                    plugin_id: id.clone(),
                    code: "required_dependency_failed",
                    message: format!(
                        "required plugin or service provider failed to activate: {}",
                        failed_requirements
                            .iter()
                            .map(|dependency| format!("`{dependency}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    dependencies: failed_requirements,
                };
                activation_blocks.insert(id.clone(), block.clone());
                record_activation_block(&statuses_shared, &logs_shared, &block);
                continue;
            }
            plugin_indices
                .write()
                .map_err(|_| HostError::Config("plugin index registry lock poisoned".into()))?
                .insert(plugin_key.clone(), loaded.len());
            let Some(mut candidate) = candidates.remove(&id) else {
                continue;
            };

            if matches!(candidate, BuildCandidate::Reused(_))
                && !service_epoch_unchanged(
                    &id,
                    &service_plan.bindings,
                    &previous_service_bindings,
                    &config.list,
                    &previous_plugins,
                )
            {
                let previous_manifest = candidate.manifest().clone();
                let configured = config.list.get(&id).expect("configured plugin");
                match prepare_entry(
                    &id,
                    configured,
                    &mut static_registry,
                    Arc::clone(&host_handle),
                    &workspace_root,
                    &env_lookup,
                    &config.host.trusted_keys,
                )
                .await
                {
                    Ok(prepared) if prepared.manifest == previous_manifest => {
                        candidate = BuildCandidate::Prepared(Box::new(prepared));
                    }
                    Ok(prepared) => {
                        if let Err(error) = crate::loader::close_transport_for_plugin(
                            &id,
                            prepared.transport().as_ref(),
                        )
                        .await
                        {
                            tracing::error!(
                                plugin = %id,
                                diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                                "failed to close a re-prepared plugin whose manifest changed during reload"
                            );
                        }
                        let block = crate::activation::PluginActivationBlock {
                            plugin_id: id.clone(),
                            code: "manifest_changed_during_reload",
                            message: "plugin manifest changed while rebuilding a service-dependent consumer; retry reload to resolve a fresh graph".to_string(),
                            dependencies: Vec::new(),
                        };
                        activation_blocks.insert(id.clone(), block.clone());
                        record_activation_block(&statuses_shared, &logs_shared, &block);
                        continue;
                    }
                    Err(error) => {
                        let block = crate::activation::PluginActivationBlock {
                            plugin_id: id.clone(),
                            code: "preparation_failed",
                            message: "service dependency changed and the consumer could not be prepared again".to_string(),
                            dependencies: Vec::new(),
                        };
                        activation_blocks.insert(id.clone(), block.clone());
                        record_activation_block(&statuses_shared, &logs_shared, &block);
                        logs_shared.append(
                            &plugin_key,
                            "error",
                            "prepare",
                            error.to_string(),
                            serde_json::Value::Null,
                        );
                        continue;
                    }
                }
            }

            let (plugin, reused_transport) = match candidate {
                BuildCandidate::Reused(reused) => {
                    tracing::info!(
                        target: "agena_plugin_host",
                        plugin = %id,
                        "reusing existing plugin transport (config and dependency epochs unchanged)"
                    );
                    if let Some(previous_host) = &previous {
                        previous_host
                            .transferred_to_successor
                            .lock()
                            .await
                            .insert(plugin_key.clone());
                    }
                    reused
                        .transport
                        .attach_host(host_handle.scoped_host_client(id.clone()))
                        .await
                        .map_err(|error| HostError::Load {
                            plugin: id.clone(),
                            message: agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to attach the successor host to a reused plugin transport",
                                &error,
                            ),
                        })?;
                    if let Some(previous_status) = previous
                        .as_ref()
                        .and_then(|host| host.plugin_status_by_key(&plugin_key))
                    {
                        statuses_shared.set(previous_status);
                    }
                    (reused, true)
                }
                BuildCandidate::Prepared(prepared) => {
                    let transport = prepared.transport();
                    match activate_entry(*prepared, &host_handle, &agena_version, &workspace_root)
                        .await
                    {
                        Ok(plugin) => (Arc::new(plugin), false),
                        Err(error) => {
                            if let Err(close_error) =
                                crate::loader::close_transport_for_plugin(&id, transport.as_ref())
                                    .await
                            {
                                tracing::error!(
                                    plugin = %id,
                                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                        "plugin transport cleanup also failed after plugin initialization failed",
                                        &close_error,
                                    ),
                                    "plugin initialization and transport cleanup both failed"
                                );
                            }
                            host_handle.rollback_failed_plugin(&plugin_key).await;
                            let block = crate::activation::PluginActivationBlock {
                                plugin_id: id.clone(),
                                code: "initialization_failed",
                                message: "plugin failed during initialization".to_string(),
                                dependencies: Vec::new(),
                            };
                            activation_blocks.insert(id.clone(), block.clone());
                            record_activation_block(&statuses_shared, &logs_shared, &block);
                            logs_shared.append(
                                &plugin_key,
                                "error",
                                "init",
                                agena_failure::diagnostic::format_error_chain(&error),
                                serde_json::Value::Null,
                            );
                            continue;
                        }
                    }
                }
            };
            tool_registry_shared
                .write()
                .map_err(|_| HostError::Config("plugin tool registry lock poisoned".into()))?
                .extend_from_plugin(&plugin.key(), &plugin.manifest.tools)
                .map_err(|message| HostError::Load {
                    plugin: plugin.key().to_string(),
                    message,
                })?;
            if let Err(error) = host_handle.own_manifest_resources(&plugin.key(), &plugin.manifest)
            {
                if let Err(close_error) =
                    crate::loader::close_transport_for_plugin(&id, plugin.transport().as_ref())
                        .await
                {
                    tracing::error!(
                        plugin = %id,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "plugin transport cleanup also failed after effect-scope registration failed",
                            &close_error,
                        ),
                        "plugin effect-scope registration and transport cleanup both failed"
                    );
                }
                host_handle.rollback_failed_plugin(&plugin.key()).await;
                let block = crate::activation::PluginActivationBlock {
                    plugin_id: id.clone(),
                    code: "effect_scope_registration_failed",
                    message: format!(
                        "plugin resources could not be assigned to an effect scope: {error}"
                    ),
                    dependencies: Vec::new(),
                };
                activation_blocks.insert(id.clone(), block.clone());
                record_activation_block(&statuses_shared, &logs_shared, &block);
                continue;
            }
            plugin_names
                .write()
                .map_err(|_| HostError::Config("plugin name registry lock poisoned".into()))?
                .insert(plugin.key(), plugin.manifest.name.clone());
            host_handle.set_plugin_hook_catalog(hook_registration_for_plugin(&plugin));
            if !reused_transport {
                statuses_shared.set(crate::status::PluginStatus::initial(
                    &plugin.key(),
                    plugin.kind,
                ));
            }
            by_id.insert(plugin.key(), Arc::clone(&plugin));
            host_handle
                .register_plugin_transport(plugin.key(), plugin.transport())
                .await;
            loaded.push(plugin);
            activated.insert(id);
        }

        // Prepared transports excluded by a transitive blocker never reached
        // init and must still be closed explicitly.
        for candidate in candidates.into_values() {
            if let BuildCandidate::Prepared(prepared) = candidate {
                let plugin_key = prepared.key();
                let transport = prepared.transport();
                let plugin_id = plugin_key.to_string();
                let close_result =
                    crate::loader::close_transport_for_plugin(&plugin_id, transport.as_ref()).await;
                host_handle.dispose_plugin_resources(&plugin_key).await;
                close_result?;
            }
        }

        let operation_registry = host_handle.operation_registry();
        let tool_before_pipeline =
            Arc::new(crate::event_pipeline::PluginTransformBailPipeline::new(
                crate::event_pipeline::PluginPipelineFailurePolicy::Abort,
            ));
        let tool_before_timeout = config
            .host
            .timeouts
            .tool_hook_or(std::time::Duration::from_secs(30));
        for plugin in &loaded {
            if !plugin.subscribes(HookSubscription::TOOL_BEFORE) {
                continue;
            }
            let plugin_id = plugin.key();
            let scope = host_handle.effect_scope(&plugin_id).ok_or_else(|| {
                HostError::Config(format!(
                    "loaded plugin `{plugin_id}` has no active effect scope for tool.before"
                ))
            })?;
            let plugin = Arc::clone(plugin);
            let callback_host_handle = Arc::clone(&host_handle);
            tool_before_pipeline
                .register(
                    &scope,
                    0,
                    format!("{plugin_id} tool.before"),
                    move |mut dispatch: ToolBeforeDispatch| {
                        let plugin = Arc::clone(&plugin);
                        let host_handle = Arc::clone(&callback_host_handle);
                        async move {
                            let params = match serde_json::to_value(&dispatch.input) {
                                Ok(params) => params,
                                Err(error) => {
                                    return Ok(
                                        crate::event_pipeline::PluginTransformBailControl::Bail(
                                            ToolBeforeBail::Error(PluginError::invalid_params(
                                                error.to_string(),
                                            )),
                                        ),
                                    );
                                }
                            };
                            let context = tool_hook_context(
                                &plugin,
                                dispatch.input.tool_name(),
                                Some(dispatch.input.session_id),
                                Some(dispatch.input.call_id),
                                Some(dispatch.input.workspace_root.clone()),
                            );
                            let value = match await_build_transport_with_cancellation(
                                dispatch.cancellation.clone(),
                                host_handle.run_in_authorized_callback_context(
                                    &plugin.key(),
                                    context,
                                    call_with_timeout(
                                        &plugin,
                                        method::HOOK_TOOL_BEFORE,
                                        params,
                                        tool_before_timeout,
                                    ),
                                ),
                            )
                            .await
                            {
                                Ok(value) => value,
                                Err(error) => {
                                    return Ok(
                                        crate::event_pipeline::PluginTransformBailControl::Bail(
                                            ToolBeforeBail::Error(transport_to_plugin_error(error)),
                                        ),
                                    );
                                }
                            };
                            if matches!(&value, serde_json::Value::Null) {
                                return Ok(
                                    crate::event_pipeline::PluginTransformBailControl::Continue(
                                        dispatch,
                                    ),
                                );
                            }
                            let patch: Option<ToolBeforePatch> = match serde_json::from_value(value)
                            {
                                Ok(patch) => patch,
                                Err(error) => {
                                    return Ok(
                                        crate::event_pipeline::PluginTransformBailControl::Bail(
                                            ToolBeforeBail::Error(PluginError::invalid_params(
                                                error.to_string(),
                                            )),
                                        ),
                                    );
                                }
                            };
                            let Some(mut patch) = patch else {
                                return Ok(
                                    crate::event_pipeline::PluginTransformBailControl::Continue(
                                        dispatch,
                                    ),
                                );
                            };
                            if let Some(reason) = patch.abort_reason.take() {
                                return Ok(
                                    crate::event_pipeline::PluginTransformBailControl::Bail(
                                        ToolBeforeBail::Abort(reason),
                                    ),
                                );
                            }
                            if let Some(input) = patch.input {
                                dispatch.input.input = input;
                            }
                            if let Some(title) = patch.title_override {
                                dispatch.input.title_override = Some(title);
                            }
                            dispatch.input.metadata.extend(patch.metadata);
                            Ok(crate::event_pipeline::PluginTransformBailControl::Continue(
                                dispatch,
                            ))
                        }
                    },
                )
                .map_err(|error| {
                    HostError::Config(agena_failure::diagnostic::format_error_chain(&error))
                })?;
        }
        let tool_after_pipeline = Arc::new(crate::event_pipeline::PluginTransformPipeline::new(
            crate::event_pipeline::PluginPipelineFailurePolicy::Abort,
        ));
        let tool_after_timeout = config
            .host
            .timeouts
            .tool_hook_or(std::time::Duration::from_secs(30));
        for plugin in &loaded {
            if !plugin.subscribes(HookSubscription::TOOL_AFTER) {
                continue;
            }
            let plugin_id = plugin.key();
            let scope = host_handle.effect_scope(&plugin_id).ok_or_else(|| {
                HostError::Config(format!(
                    "loaded plugin `{plugin_id}` has no active effect scope for tool.after"
                ))
            })?;
            let plugin = Arc::clone(plugin);
            let callback_host_handle = Arc::clone(&host_handle);
            tool_after_pipeline
                .register(
                    &scope,
                    0,
                    format!("{plugin_id} tool.after"),
                    move |mut dispatch: ToolAfterDispatch| {
                        let plugin = Arc::clone(&plugin);
                        let host_handle = Arc::clone(&callback_host_handle);
                        async move {
                            let params =
                                serde_json::to_value(&dispatch.input).map_err(|error| {
                                    agena_failure::diagnostic::format_error_chain_with_context(
                                        "failed to serialize tool.after hook input",
                                        &error,
                                    )
                                })?;
                            let context = tool_hook_context(
                                &plugin,
                                dispatch.input.tool_name(),
                                Some(dispatch.input.session_id),
                                Some(dispatch.input.call_id),
                                Some(dispatch.input.workspace_root.clone()),
                            );
                            let value = await_build_transport_with_cancellation(
                                dispatch.cancellation.clone(),
                                host_handle.run_in_authorized_callback_context(
                                    &plugin.key(),
                                    context,
                                    call_with_timeout(
                                        &plugin,
                                        method::HOOK_TOOL_AFTER,
                                        params,
                                        tool_after_timeout,
                                    ),
                                ),
                            )
                            .await
                            .map_err(|error| {
                                let error = transport_to_plugin_error(error);
                                error.diagnostic_message().to_owned()
                            })?;
                            if value.is_null() {
                                return Ok(dispatch);
                            }
                            let patch: Option<ToolAfterPatch> = serde_json::from_value(value)
                                .map_err(|error| {
                                    agena_failure::diagnostic::format_error_chain_with_context(
                                        "failed to decode tool.after hook output",
                                        &error,
                                    )
                                })?;
                            if let Some(patch) = patch {
                                if let Some(title) = patch.title {
                                    dispatch.input.title = title;
                                }
                                if let Some(summary) = patch.summary {
                                    dispatch.input.summary = summary;
                                }
                                if let Some(output_text) = patch.output_text {
                                    dispatch.input.output_text = output_text;
                                }
                                if let Some(payload) = patch.payload {
                                    dispatch.input.payload = Some(payload);
                                }
                                dispatch.input.metadata.extend(patch.metadata);
                            }
                            Ok(dispatch)
                        }
                    },
                )
                .map_err(|error| {
                    HostError::Config(agena_failure::diagnostic::format_error_chain(&error))
                })?;
        }
        Ok(Arc::new(PluginHost {
            plugins: loaded,
            plugins_by_id: by_id,
            tool_registry: tool_registry_shared,
            operation_registry,
            operation_pipeline: Arc::new(crate::event_pipeline::PluginAroundPipeline::new()),
            tool_before_pipeline,
            tool_after_pipeline,
            statuses: statuses_shared,
            logs: logs_shared,
            configured_plugins: configured_plugins_for_inspect,
            activation_blocks,
            activation_epochs,
            reload_plan,
            profile_resolution,
            prefetched_manifests,
            service_bindings: service_plan.bindings,
            timeouts: config.host.timeouts,
            _host_handle: host_handle,
            transferred_to_successor: tokio::sync::Mutex::new(Default::default()),
            hook_runs: Arc::new(std::sync::Mutex::new(Vec::new())),
        }))
    }
}

async fn await_build_transport_with_cancellation<T, F>(
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

fn record_activation_block(
    statuses: &Arc<crate::status::StatusRegistry>,
    logs: &Arc<PluginLogStore>,
    block: &crate::activation::PluginActivationBlock,
) {
    let Ok(plugin_key) = block.plugin_id.parse::<PluginKey>() else {
        return;
    };
    statuses.record_spawn_failure(&plugin_key, block.message.clone());
    logs.append(
        &plugin_key,
        "error",
        "activation",
        block.message.clone(),
        serde_json::json!({
            "code": block.code,
            "dependencies": block.dependencies,
        }),
    );
    tracing::warn!(
        target: "agena_plugin_host",
        plugin = %block.plugin_id,
        code = block.code,
        dependencies = ?block.dependencies,
        "plugin activation blocked: {}",
        block.message
    );
}

fn service_epoch_changed_providers(
    consumer: &str,
    current: &BTreeMap<PluginServiceBindingKey, PluginServiceBinding>,
    previous: &BTreeMap<PluginServiceBindingKey, PluginServiceBinding>,
    current_config: &BTreeMap<String, crate::config::ConfiguredPlugin>,
    previous_config: &HashMap<String, crate::config::ConfiguredPlugin>,
) -> BTreeSet<String> {
    let current_bindings = current
        .iter()
        .filter(|(key, binding)| key.consumer == consumer && !binding.optional)
        .map(|(key, binding)| (key.clone(), binding.clone()))
        .collect::<BTreeMap<_, _>>();
    let previous_bindings = previous
        .iter()
        .filter(|(key, binding)| key.consumer == consumer && !binding.optional)
        .map(|(key, binding)| (key.clone(), binding.clone()))
        .collect::<BTreeMap<_, _>>();
    let keys = current_bindings
        .keys()
        .chain(previous_bindings.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut changed = BTreeSet::new();
    for key in keys {
        let current_binding = current_bindings.get(&key);
        let previous_binding = previous_bindings.get(&key);
        if current_binding != previous_binding {
            if let Some(binding) = current_binding {
                changed.insert(binding.provider.clone());
            }
            if let Some(binding) = previous_binding {
                changed.insert(binding.provider.clone());
            }
            continue;
        }
        if let Some(binding) = current_binding
            && previous_config.get(&binding.provider) != current_config.get(&binding.provider)
        {
            changed.insert(binding.provider.clone());
        }
    }
    changed
}

fn service_epoch_unchanged(
    consumer: &str,
    current: &BTreeMap<PluginServiceBindingKey, PluginServiceBinding>,
    previous: &BTreeMap<PluginServiceBindingKey, PluginServiceBinding>,
    current_config: &BTreeMap<String, crate::config::ConfiguredPlugin>,
    previous_config: &HashMap<String, crate::config::ConfiguredPlugin>,
) -> bool {
    service_epoch_changed_providers(consumer, current, previous, current_config, previous_config)
        .is_empty()
}

use super::{
    Arc, BTreeMap, BTreeSet, HashMap, HookSubscription, HostError, HostHandle, LoadedPlugin,
    NoopHostClient, PluginError, PluginHost, PluginHostBuildConfig, PluginKey, PluginLogStore,
    PluginPackage, PluginServiceBinding, PluginServiceBindingKey, PluginToolRegistry,
    PreparedPlugin, RwLock, StaticRegistration, ToolAfterDispatch, ToolAfterPatch, ToolBeforeBail,
    ToolBeforeDispatch, ToolBeforePatch, TransportError, activate_entry, call_with_timeout,
    hook_registration_for_plugin, method, prepare_entry, tool_hook_context,
    transport_to_plugin_error,
};

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};

    use crate::config::{ConfiguredPlugin, PluginActivationConfig, PluginsConfig};
    use crate::sdk::{
        HookSubscription, HostClient, InitContext, InitOutcome, Plugin, PluginError, PluginKey,
        PluginManifest, PluginServiceClient, PluginServiceExport, PluginServiceInvokeExt,
        PluginServiceInvokeInput, PluginServiceMethod, SettingsConstraints, SettingsContract,
        SettingsNode, SettingsNodeKind, ToolBeforeInput, ToolBeforePatch, encode_service_output,
    };

    use super::{PluginHost, PluginHostBuildConfig, PluginServiceBinding, PluginServiceBindingKey};
    use crate::host::StaticPluginRegistration;

    struct CommandBeforeContextPlugin {
        manifest: PluginManifest,
        contexts: Arc<Mutex<Vec<crate::sdk::host_api::HostCallbackContext>>>,
    }

    impl CommandBeforeContextPlugin {
        fn new(contexts: Arc<Mutex<Vec<crate::sdk::host_api::HostCallbackContext>>>) -> Self {
            let mut manifest = PluginManifest::new("example", "command-context", "0.1.0");
            manifest.hooks = HookSubscription::COMMAND_BEFORE | HookSubscription::COMMAND_AFTER;
            Self { manifest, contexts }
        }
    }

    #[async_trait]
    impl Plugin for CommandBeforeContextPlugin {
        fn manifest(&self) -> PluginManifest {
            self.manifest.clone()
        }

        async fn init(
            &self,
            _ctx: InitContext,
            _host: Arc<dyn HostClient>,
        ) -> crate::sdk::Result<InitOutcome> {
            Ok(InitOutcome::ack(self.manifest()))
        }

        async fn command_execute_before(
            &self,
            _input: crate::sdk::CommandBeforeInput,
        ) -> crate::sdk::Result<Option<crate::sdk::CommandBeforeResponse>> {
            self.contexts
                .lock()
                .expect("command hook context lock")
                .push(crate::sdk::host_api::current_host_callback_context().unwrap_or_default());
            Ok(None)
        }

        async fn command_execute_after(
            &self,
            _input: crate::sdk::CommandAfterInput,
        ) -> crate::sdk::Result<Option<crate::sdk::CommandAfterPatch>> {
            self.contexts
                .lock()
                .expect("command hook context lock")
                .push(crate::sdk::host_api::current_host_callback_context().unwrap_or_default());
            Ok(None)
        }
    }

    struct RecordingPlugin {
        manifest: PluginManifest,
        events: Arc<Mutex<Vec<String>>>,
        fail_init: bool,
    }

    impl RecordingPlugin {
        fn new(
            namespace: &str,
            name: &str,
            events: Arc<Mutex<Vec<String>>>,
            fail_init: bool,
        ) -> Self {
            Self {
                manifest: PluginManifest::new(namespace, name, "0.1.0"),
                events,
                fail_init,
            }
        }
    }

    #[async_trait]
    impl Plugin for RecordingPlugin {
        fn manifest(&self) -> PluginManifest {
            self.manifest.clone()
        }

        async fn init(
            &self,
            ctx: InitContext,
            _host: Arc<dyn HostClient>,
        ) -> crate::sdk::Result<InitOutcome> {
            self.events
                .lock()
                .expect("recording plugin event lock")
                .push(ctx.plugin_id.to_string());
            if self.fail_init {
                return Err(PluginError::internal("intentional provider init failure"));
            }
            Ok(InitOutcome::ack(self.manifest()))
        }
    }

    #[derive(Clone, Copy)]
    enum ToolBeforeBehavior {
        Patch,
        Abort,
        Observe,
    }

    struct ToolBeforePlugin {
        manifest: PluginManifest,
        events: Arc<Mutex<Vec<String>>>,
        label: &'static str,
        behavior: ToolBeforeBehavior,
    }

    impl ToolBeforePlugin {
        fn new(
            name: &str,
            label: &'static str,
            behavior: ToolBeforeBehavior,
            events: Arc<Mutex<Vec<String>>>,
        ) -> Self {
            let mut manifest = PluginManifest::new("example", name, "0.1.0");
            manifest.hooks = HookSubscription::TOOL_BEFORE;
            Self {
                manifest,
                events,
                label,
                behavior,
            }
        }
    }

    #[async_trait]
    impl Plugin for ToolBeforePlugin {
        fn manifest(&self) -> PluginManifest {
            self.manifest.clone()
        }

        async fn tool_execute_before(
            &self,
            input: ToolBeforeInput,
        ) -> crate::sdk::Result<Option<ToolBeforePatch>> {
            let seen = input
                .input
                .get("stage")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("initial");
            self.events
                .lock()
                .expect("tool before events")
                .push(format!("{}:{seen}", self.label));
            Ok(match self.behavior {
                ToolBeforeBehavior::Patch => Some(ToolBeforePatch {
                    input: Some(serde_json::json!({"stage": self.label})),
                    title_override: Some(format!("{} title", self.label)),
                    metadata: std::collections::BTreeMap::from([(
                        self.label.to_string(),
                        "seen".to_string(),
                    )]),
                    ..ToolBeforePatch::default()
                }),
                ToolBeforeBehavior::Abort => Some(ToolBeforePatch {
                    abort_reason: Some(format!("stop at {}", self.label)),
                    ..ToolBeforePatch::default()
                }),
                ToolBeforeBehavior::Observe => None,
            })
        }
    }

    fn configured(requires: &[&str]) -> ConfiguredPlugin {
        ConfiguredPlugin {
            activation: PluginActivationConfig {
                requires: requires
                    .iter()
                    .map(|value| value.parse().expect("valid dependency id"))
                    .collect(),
                after: Vec::new(),
            },
            ..ConfiguredPlugin::static_default()
        }
    }

    #[tokio::test]
    async fn command_hooks_receive_host_issued_callback_authority() {
        let contexts = Arc::new(Mutex::new(Vec::new()));
        let plugin_key: PluginKey = "example.command-context".parse().expect("plugin key");
        let mut config = PluginsConfig::default();
        config.list.insert(plugin_key.to_string(), configured(&[]));
        let host = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![StaticPluginRegistration::new(
                plugin_key,
                CommandBeforeContextPlugin::new(Arc::clone(&contexts)),
            )],
            config,
            workspace_root: PathBuf::from("/tmp/agena-command-context-test"),
            agena_version: "0.1.0".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .expect("build command context host");

        let outcome = host
            .dispatch_command_before(crate::sdk::CommandBeforeInput {
                session_id: Some(41),
                call_id: Some(73),
                workspace_root: Some("/tmp/agena-workspace".to_string()),
                command: "sh".to_string(),
                args: vec!["-c".to_string(), "git status".to_string()],
                cwd: PathBuf::from("/tmp/agena-workspace"),
                env: BTreeMap::new(),
            })
            .await
            .expect("dispatch command-before hook");
        assert!(matches!(
            outcome,
            crate::sdk::CommandBeforeOutcome::Continue(_)
        ));

        {
            let seen = contexts.lock().expect("command-before context lock");
            assert_eq!(seen.len(), 1);
            let context = &seen[0];
            assert_eq!(
                context.plugin_id.as_deref(),
                Some("example.command-context")
            );
            assert_eq!(context.session_id, Some(41));
            assert_eq!(context.call_id, Some(73));
            assert_eq!(
                context.workspace_root.as_deref(),
                Some("/tmp/agena-workspace")
            );
            assert!(
                context
                    .authority_token
                    .as_deref()
                    .is_some_and(|token| token.starts_with("ctx-")),
                "command.before must inherit a host-issued callback authority"
            );
        }

        host.dispatch_command_after(crate::sdk::CommandAfterInput {
            session_id: Some(41),
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "git status".to_string()],
            cwd: PathBuf::from("/tmp/agena-workspace"),
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
        })
        .await
        .expect("dispatch command-after hook");
        {
            let seen = contexts.lock().expect("command hook context lock");
            assert_eq!(seen.len(), 2);
            let context = &seen[1];
            assert_eq!(
                context.plugin_id.as_deref(),
                Some("example.command-context")
            );
            assert_eq!(context.session_id, Some(41));
            assert!(
                context
                    .authority_token
                    .as_deref()
                    .is_some_and(|token| token.starts_with("ctx-")),
                "command.after must inherit a host-issued callback authority"
            );
        }
        host.shutdown().await;
    }

    async fn build_recording_host(
        events: Arc<Mutex<Vec<String>>>,
        fail_provider: bool,
    ) -> Arc<PluginHost> {
        let provider_key: PluginKey = "example.provider".parse().expect("provider key");
        let consumer_key: PluginKey = "example.consumer".parse().expect("consumer key");
        let mut config = PluginsConfig::default();
        // Insert consumer first to prove map/insertion order does not control
        // activation.
        config
            .list
            .insert(consumer_key.to_string(), configured(&["example.provider"]));
        config
            .list
            .insert(provider_key.to_string(), configured(&[]));

        PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![
                StaticPluginRegistration::new(
                    consumer_key,
                    RecordingPlugin::new("example", "consumer", Arc::clone(&events), false),
                ),
                StaticPluginRegistration::new(
                    provider_key,
                    RecordingPlugin::new("example", "provider", events, fail_provider),
                ),
            ],
            config,
            workspace_root: PathBuf::from("/tmp/agena-plugin-activation-test"),
            agena_version: "0.1.0".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .expect("build plugin host")
    }

    #[tokio::test]
    async fn tool_before_pipeline_transforms_in_order_and_bails_before_later_plugins() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut config = PluginsConfig::default();
        let registrations = [
            (
                "example.alpha",
                ToolBeforePlugin::new(
                    "alpha",
                    "alpha",
                    ToolBeforeBehavior::Patch,
                    Arc::clone(&events),
                ),
            ),
            (
                "example.beta",
                ToolBeforePlugin::new(
                    "beta",
                    "beta",
                    ToolBeforeBehavior::Abort,
                    Arc::clone(&events),
                ),
            ),
            (
                "example.gamma",
                ToolBeforePlugin::new(
                    "gamma",
                    "gamma",
                    ToolBeforeBehavior::Observe,
                    Arc::clone(&events),
                ),
            ),
        ];
        let mut static_plugins = Vec::new();
        for (plugin_id, plugin) in registrations {
            let key: PluginKey = plugin_id.parse().expect("plugin key");
            config.list.insert(plugin_id.to_string(), configured(&[]));
            static_plugins.push(StaticPluginRegistration::new(key, plugin));
        }
        let host = PluginHost::new(PluginHostBuildConfig {
            static_plugins,
            config,
            workspace_root: PathBuf::from("/tmp/agena-tool-before-pipeline-test"),
            agena_version: "0.1.0".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .expect("build tool-before host");

        let error = host
            .dispatch_tool_before(
                ToolBeforeInput {
                    tool: "target.plugin.run".parse().expect("tool key"),
                    session_id: 1,
                    call_id: 2,
                    workspace_root: "/workspace".to_string(),
                    tags: Vec::new(),
                    contract: Default::default(),
                    input: serde_json::json!({}),
                    title_override: None,
                    metadata: Default::default(),
                },
                None,
            )
            .await
            .expect_err("beta aborts the pipeline");

        assert!(error.to_string().contains("stop at beta"));
        assert_eq!(
            events.lock().expect("events").as_slice(),
            ["alpha:initial", "beta:alpha"]
        );
        host.shutdown().await;
    }

    #[tokio::test]
    async fn host_initializes_required_provider_before_consumer() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let host = build_recording_host(Arc::clone(&events), false).await;

        assert_eq!(
            events.lock().expect("event lock").as_slice(),
            ["example.provider", "example.consumer"]
        );
        assert_eq!(host.plugins().len(), 2);
        host.shutdown().await;
    }

    #[tokio::test]
    async fn failed_provider_prevents_consumer_initialization() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let host = build_recording_host(Arc::clone(&events), true).await;

        assert_eq!(
            events.lock().expect("event lock").as_slice(),
            ["example.provider"]
        );
        let consumer = host
            .plugin_status("example.consumer")
            .expect("consumer status remains inspectable");
        assert_eq!(consumer.state, crate::status::PluginRunState::Failed);
        assert!(consumer.last_failure.is_some());
        let logs = host.plugin_logs("example.consumer", None, 10);
        assert!(
            logs.iter().any(|entry| {
                entry.source == "activation"
                    && entry.message.contains("failed to activate")
                    && entry.fields["dependencies"][0] == "example.provider"
            }),
            "consumer activation log should identify the failed requirement: {logs:#?}"
        );
        let inspect = host
            .plugin_inspect("example.consumer")
            .expect("blocked consumer remains inspectable");
        let activation = inspect.activation.expect("activation projection");
        assert_eq!(
            activation
                .requires
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["example.provider"]
        );
        let blocker = activation.blocked.expect("structured activation blocker");
        assert_eq!(blocker.code, "required_dependency_failed");
        assert_eq!(
            blocker
                .dependencies
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["example.provider"]
        );
        assert!(inspect.configured_plugin.is_some());
        assert!(host.plugins().is_empty());
        host.shutdown().await;
    }

    struct ServiceProviderPlugin {
        manifest: PluginManifest,
        events: Arc<Mutex<Vec<String>>>,
    }

    #[derive(Debug, Serialize, Deserialize, crate::sdk::JsonSchema)]
    struct SearchRequest {
        q: String,
    }

    #[derive(Debug, Serialize, Deserialize, crate::sdk::JsonSchema)]
    struct SearchResponse {
        echo: SearchRequest,
        provider: String,
    }

    crate::sdk::plugin_service_endpoint! {
        SearchQuery {
            service: "workspace.search",
            version: 1,
            method: "query",
            input: SearchRequest,
            output: SearchResponse,
        }
    }

    struct GeneratedServiceProviderPlugin {
        events: Arc<Mutex<Vec<String>>>,
    }

    #[crate::sdk::agena_plugin(
        namespace = "example",
        name = "generated_provider",
        version = "0.1.0",
        summary = "Macro-generated typed service provider fixture."
    )]
    impl GeneratedServiceProviderPlugin {
        #[hook(init)]
        async fn init(
            &self,
            ctx: InitContext,
            _host: Arc<dyn HostClient>,
        ) -> crate::sdk::Result<InitOutcome> {
            self.events
                .lock()
                .expect("service event lock")
                .push(format!("init:{}", ctx.plugin_id));
            Ok(InitOutcome::ack(crate::sdk::Plugin::manifest(self)))
        }

        #[service(SearchQuery)]
        fn query(&self, input: &SearchRequest) -> SearchResponse {
            self.events
                .lock()
                .expect("service event lock")
                .push("call:workspace.search::query".to_string());
            SearchResponse {
                echo: SearchRequest { q: input.q.clone() },
                provider: "example.generated_provider".to_string(),
            }
        }
    }

    #[async_trait]
    impl Plugin for ServiceProviderPlugin {
        fn manifest(&self) -> PluginManifest {
            self.manifest.clone()
        }

        async fn init(
            &self,
            ctx: InitContext,
            _host: Arc<dyn HostClient>,
        ) -> crate::sdk::Result<InitOutcome> {
            self.events
                .lock()
                .expect("service event lock")
                .push(format!("init:{}", ctx.plugin_id));
            Ok(InitOutcome::ack(self.manifest()))
        }

        async fn service_invoke(
            &self,
            input: PluginServiceInvokeInput,
        ) -> crate::sdk::Result<serde_json::Value> {
            self.events
                .lock()
                .expect("service event lock")
                .push(format!("call:{}::{}", input.service, input.method));
            if input.method == "strict_output" {
                return Ok(serde_json::json!({"not_ok": true}));
            }
            let request: SearchRequest = input.decode()?;
            encode_service_output(SearchResponse {
                echo: request,
                provider: "example.provider".to_string(),
            })
        }
    }

    struct ServiceConsumerPlugin {
        manifest: PluginManifest,
        events: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Plugin for ServiceConsumerPlugin {
        fn manifest(&self) -> PluginManifest {
            self.manifest.clone()
        }

        async fn init(
            &self,
            ctx: InitContext,
            host: Arc<dyn HostClient>,
        ) -> crate::sdk::Result<InitOutcome> {
            self.events
                .lock()
                .expect("service event lock")
                .push(format!("init:{}", ctx.plugin_id));
            let client = PluginServiceClient::endpoint::<SearchQuery>(host);
            let output = client
                .call(&SearchRequest {
                    q: "cordis".to_string(),
                })
                .await?;
            self.events
                .lock()
                .expect("service event lock")
                .push(format!(
                    "result:{}:{}",
                    output.provider, output.output.echo.q
                ));
            Ok(InitOutcome::ack(self.manifest()))
        }
    }

    fn service_text_field(id: &str, path: &str, title: &str) -> SettingsNode {
        SettingsNode {
            id: id.to_string(),
            path: path.to_string(),
            title: title.to_string(),
            description: String::new(),
            required: true,
            default: None,
            constraints: SettingsConstraints {
                min_length: Some(1),
                ..SettingsConstraints::default()
            },
            sensitive: false,
            secret: false,
            kind: SettingsNodeKind::Text,
        }
    }

    fn search_query_method() -> PluginServiceMethod {
        PluginServiceMethod::new(
            "query",
            SettingsContract::new(SettingsNode {
                id: "root".to_string(),
                path: String::new(),
                title: "Search query".to_string(),
                description: String::new(),
                required: true,
                default: None,
                constraints: SettingsConstraints::default(),
                sensitive: false,
                secret: false,
                kind: SettingsNodeKind::Object {
                    fields: vec![service_text_field("q", "/q", "Query")],
                },
            }),
            SettingsContract::bounded_json("Search result", "", 65_536, 16),
        )
    }

    fn service_provider_manifest(id: &str) -> PluginManifest {
        let (namespace, name) = id.split_once('.').expect("test plugin id");
        let mut manifest = PluginManifest::new(namespace, name, "0.1.0");
        manifest.services.exports = vec![
            PluginServiceExport::new("workspace.search", 1)
                .with_method(search_query_method())
                .with_method(PluginServiceMethod::new(
                    "strict_output",
                    SettingsContract::empty_object("No input", ""),
                    SettingsContract::new(SettingsNode {
                        id: "root".to_string(),
                        path: String::new(),
                        title: "Strict output".to_string(),
                        description: String::new(),
                        required: true,
                        default: None,
                        constraints: SettingsConstraints::default(),
                        sensitive: false,
                        secret: false,
                        kind: SettingsNodeKind::Object {
                            fields: vec![SettingsNode {
                                id: "ok".to_string(),
                                path: "/ok".to_string(),
                                title: "OK".to_string(),
                                description: String::new(),
                                required: true,
                                default: None,
                                constraints: SettingsConstraints::default(),
                                sensitive: false,
                                secret: false,
                                kind: SettingsNodeKind::Boolean,
                            }],
                        },
                    }),
                )),
        ];
        manifest
    }

    fn service_consumer_manifest(optional: bool) -> PluginManifest {
        let mut manifest = PluginManifest::new("example", "consumer", "0.1.0");
        manifest.services.imports = vec![if optional {
            SearchQuery::optional_import()
        } else {
            SearchQuery::required_import()
        }];
        manifest
    }

    #[tokio::test]
    async fn declared_service_provider_is_active_before_consumer_init_call() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let consumer_key: PluginKey = "example.consumer".parse().expect("consumer key");
        let provider_key: PluginKey = "example.provider".parse().expect("provider key");
        let mut config = PluginsConfig::default();
        config
            .list
            .insert(consumer_key.to_string(), configured(&[]));
        config
            .list
            .insert(provider_key.to_string(), configured(&[]));

        let host = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![
                StaticPluginRegistration::new(
                    consumer_key,
                    ServiceConsumerPlugin {
                        manifest: service_consumer_manifest(false),
                        events: Arc::clone(&events),
                    },
                ),
                StaticPluginRegistration::new(
                    provider_key,
                    ServiceProviderPlugin {
                        manifest: service_provider_manifest("example.provider"),
                        events: Arc::clone(&events),
                    },
                ),
            ],
            config,
            workspace_root: PathBuf::from("/tmp/agena-plugin-service-test"),
            agena_version: "0.1.0".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .expect("build service-aware plugin host");

        assert_eq!(
            events.lock().expect("service event lock").as_slice(),
            [
                "init:example.provider",
                "init:example.consumer",
                "call:workspace.search::query",
                "result:example.provider:cordis",
            ]
        );
        let inspect = host
            .plugin_inspect("example.consumer")
            .expect("consumer inspect");
        let services = inspect.services.expect("service inspect");
        assert_eq!(services.imports[0].state, "bound");
        assert_eq!(
            services.imports[0].resolved_provider.as_deref(),
            Some("example.provider")
        );
        let effects = inspect.effects.expect("consumer effect scope");
        assert_eq!(
            effects.lifecycle,
            crate::effect_scope::PluginEffectScopeState::Active
        );
        let effect_kinds = effects
            .effects
            .iter()
            .map(|effect| effect.kind.as_str())
            .collect::<BTreeSet<_>>();
        assert!(effect_kinds.contains("service.import"));
        assert!(effect_kinds.contains("host.hooks"));
        assert!(effect_kinds.contains("host.transport"));
        let provider_effects = host
            .plugin_inspect("example.provider")
            .and_then(|inspect| inspect.effects)
            .expect("provider effect scope");
        assert!(
            provider_effects
                .effects
                .iter()
                .any(|effect| effect.kind == "service.export")
        );
        host.shutdown().await;
        let disposed = host
            .plugin_inspect("example.consumer")
            .and_then(|inspect| inspect.effects)
            .expect("disposed consumer effect scope");
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
    async fn method_level_service_macro_runs_through_real_host_binding_and_validation() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let consumer_key: PluginKey = "example.consumer".parse().expect("consumer key");
        let provider_key: PluginKey = "example.generated_provider".parse().expect("provider key");
        let mut config = PluginsConfig::default();
        config
            .list
            .insert(consumer_key.to_string(), configured(&[]));
        config
            .list
            .insert(provider_key.to_string(), configured(&[]));

        let host = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![
                StaticPluginRegistration::new(
                    consumer_key,
                    ServiceConsumerPlugin {
                        manifest: service_consumer_manifest(false),
                        events: Arc::clone(&events),
                    },
                ),
                StaticPluginRegistration::new(
                    provider_key,
                    GeneratedServiceProviderPlugin {
                        events: Arc::clone(&events),
                    },
                ),
            ],
            config,
            workspace_root: PathBuf::from("/tmp/agena-generated-service-test"),
            agena_version: "0.1.0".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .expect("build host with macro-generated service provider");

        assert_eq!(
            events.lock().expect("service event lock").as_slice(),
            [
                "init:example.generated_provider",
                "init:example.consumer",
                "call:workspace.search::query",
                "result:example.generated_provider:cordis",
            ],
            "consumer={:#?}\nconsumer_logs={:#?}\nprovider={:#?}",
            host.plugin_inspect("example.consumer"),
            host.plugin_logs("example.consumer", None, 50),
            host.plugin_inspect("example.generated_provider"),
        );
        let provider = host
            .plugin_inspect("example.generated_provider")
            .and_then(|inspect| inspect.services)
            .expect("generated provider service inspect");
        assert_eq!(provider.exports.len(), 1);
        assert_eq!(provider.exports[0].id, "workspace.search");
        assert_eq!(provider.exports[0].methods[0].id, "query");
        host.shutdown().await;
    }

    #[tokio::test]
    async fn service_method_contract_is_enforced_at_the_host_boundary() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let consumer_key: PluginKey = "example.consumer".parse().expect("consumer key");
        let provider_key: PluginKey = "example.provider".parse().expect("provider key");
        let mut config = PluginsConfig::default();
        config
            .list
            .insert(consumer_key.to_string(), configured(&[]));
        config
            .list
            .insert(provider_key.to_string(), configured(&[]));
        let host = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![
                StaticPluginRegistration::new(
                    consumer_key,
                    ServiceConsumerPlugin {
                        manifest: service_consumer_manifest(false),
                        events: Arc::clone(&events),
                    },
                ),
                StaticPluginRegistration::new(
                    provider_key,
                    ServiceProviderPlugin {
                        manifest: service_provider_manifest("example.provider"),
                        events: Arc::clone(&events),
                    },
                ),
            ],
            config,
            workspace_root: PathBuf::from("/tmp/agena-plugin-service-contract-test"),
            agena_version: "0.1.0".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .expect("build service contract host");

        let calls_before = events.lock().expect("service event lock").len();
        let invalid = host
            .host_handle()
            .invoke_service_for_plugin(
                "example.consumer",
                PluginServiceInvokeInput {
                    service: "workspace.search".to_string(),
                    api_version: 1,
                    method: "query".to_string(),
                    input: serde_json::json!({"q":""}),
                },
                None,
            )
            .await
            .expect_err("invalid service input must be rejected before dispatch");
        assert!(invalid.to_string().contains("input is invalid"));
        assert_eq!(
            events.lock().expect("service event lock").len(),
            calls_before,
            "invalid service input must not reach the provider"
        );

        let unknown = host
            .host_handle()
            .invoke_service_for_plugin(
                "example.consumer",
                PluginServiceInvokeInput {
                    service: "workspace.search".to_string(),
                    api_version: 1,
                    method: "missing".to_string(),
                    input: serde_json::json!({}),
                },
                None,
            )
            .await
            .expect_err("undeclared service method must be rejected");
        assert!(unknown.to_string().contains("does not declare method"));

        let invalid_output = host
            .host_handle()
            .invoke_service_for_plugin(
                "example.consumer",
                PluginServiceInvokeInput {
                    service: "workspace.search".to_string(),
                    api_version: 1,
                    method: "strict_output".to_string(),
                    input: serde_json::json!({}),
                },
                None,
            )
            .await
            .expect_err("invalid service output must be rejected after dispatch");
        assert!(
            invalid_output
                .to_string()
                .contains("returned invalid output")
        );

        let services = host
            .plugin_inspect("example.consumer")
            .and_then(|inspect| inspect.services)
            .expect("service inspect");
        assert_eq!(
            services.imports[0]
                .methods
                .iter()
                .map(|method| method.id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["query", "strict_output"])
        );
        host.shutdown().await;
    }

    #[tokio::test]
    async fn missing_required_service_blocks_before_init_but_keeps_manifest_inspectable() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let consumer_key: PluginKey = "example.consumer".parse().expect("consumer key");
        let mut config = PluginsConfig::default();
        config
            .list
            .insert(consumer_key.to_string(), configured(&[]));
        let host = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![StaticPluginRegistration::new(
                consumer_key,
                ServiceConsumerPlugin {
                    manifest: service_consumer_manifest(false),
                    events: Arc::clone(&events),
                },
            )],
            config,
            workspace_root: PathBuf::from("/tmp/agena-plugin-service-missing-test"),
            agena_version: "0.1.0".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .expect("build blocked service host");

        assert!(events.lock().expect("service event lock").is_empty());
        assert!(host.plugins().is_empty());
        let inspect = host
            .plugin_inspect("example.consumer")
            .expect("blocked consumer inspect");
        assert!(inspect.manifest.is_some());
        assert_eq!(
            inspect
                .activation
                .and_then(|activation| activation.blocked)
                .expect("service blocker")
                .code,
            "service_provider_missing"
        );
        assert_eq!(
            inspect.services.expect("service inspect").imports[0].state,
            "blocked"
        );
        host.shutdown().await;
    }

    #[test]
    fn service_epoch_includes_provider_binding_and_provider_config() {
        let key = PluginServiceBindingKey {
            consumer: "example.consumer".to_string(),
            service: "workspace.search".to_string(),
            api_version: 1,
        };
        let binding = PluginServiceBinding {
            consumer: key.consumer.clone(),
            provider: "example.provider".to_string(),
            service: key.service.clone(),
            api_version: 1,
            optional: false,
            methods: BTreeMap::from([(
                "query".to_string(),
                PluginServiceMethod::bounded_json("query", 65_536, 16),
            )]),
        };
        let current_bindings = BTreeMap::from([(key.clone(), binding.clone())]);
        let previous_bindings = BTreeMap::from([(key, binding)]);
        let current_config = BTreeMap::from([
            ("example.consumer".to_string(), configured(&[])),
            ("example.provider".to_string(), configured(&[])),
        ]);
        let previous_config = current_config
            .iter()
            .map(|(id, configured)| (id.clone(), configured.clone()))
            .collect::<HashMap<_, _>>();
        assert!(super::service_epoch_unchanged(
            "example.consumer",
            &current_bindings,
            &previous_bindings,
            &current_config,
            &previous_config,
        ));

        let mut changed_config = current_config.clone();
        changed_config.get_mut("example.provider").unwrap().settings =
            serde_json::json!({"epoch":2});
        assert!(!super::service_epoch_unchanged(
            "example.consumer",
            &current_bindings,
            &previous_bindings,
            &changed_config,
            &previous_config,
        ));
        assert_eq!(
            super::service_epoch_changed_providers(
                "example.consumer",
                &current_bindings,
                &previous_bindings,
                &changed_config,
                &previous_config,
            ),
            BTreeSet::from(["example.provider".to_string()])
        );

        let mut optional_binding = current_bindings[&PluginServiceBindingKey {
            consumer: "example.consumer".to_string(),
            service: "workspace.search".to_string(),
            api_version: 1,
        }]
            .clone();
        optional_binding.optional = true;
        let optional_key = PluginServiceBindingKey {
            consumer: "example.consumer".to_string(),
            service: "workspace.search".to_string(),
            api_version: 1,
        };
        let optional_current = BTreeMap::from([(optional_key.clone(), optional_binding.clone())]);
        let optional_previous = BTreeMap::from([(optional_key, optional_binding)]);
        assert!(
            super::service_epoch_unchanged(
                "example.consumer",
                &optional_current,
                &optional_previous,
                &changed_config,
                &previous_config,
            ),
            "optional services live-rebind and must not force the consumer transport to restart"
        );
    }

    struct OperationPlugin {
        manifest: PluginManifest,
    }

    impl OperationPlugin {
        fn new() -> Self {
            let mut manifest = PluginManifest::new("example", "operation", "0.1.0");
            manifest
                .operations
                .push(agena_plugin_sdk::PluginOperationDefinition {
                    id: "run".to_string(),
                    title: "Run".to_string(),
                    description: String::new(),
                    group: "Test".to_string(),
                    category: None,
                    slash: Some("/run".to_string()),
                    aliases: Vec::new(),
                    usage: None,
                    input: agena_plugin_sdk::SettingsContract::new(
                        agena_plugin_sdk::SettingsNode::root_object("Input", ""),
                    ),
                    discoverability: Default::default(),
                    target: agena_plugin_sdk::PluginOperationTarget::Method {
                        handler: "run".to_string(),
                    },
                });
            Self { manifest }
        }
    }

    #[async_trait]
    impl Plugin for OperationPlugin {
        fn manifest(&self) -> PluginManifest {
            self.manifest.clone()
        }

        async fn init(
            &self,
            _ctx: InitContext,
            _host: Arc<dyn HostClient>,
        ) -> crate::sdk::Result<InitOutcome> {
            Ok(InitOutcome::ack(self.manifest()))
        }

        async fn operation_invoke(
            &self,
            _input: agena_plugin_sdk::PluginOperationInvokeInput,
        ) -> crate::sdk::Result<agena_plugin_sdk::PluginOperationResult> {
            Ok(agena_plugin_sdk::PluginOperationResult::succeeded(
                "terminal",
            ))
        }
    }

    #[tokio::test]
    async fn operation_dispatch_runs_through_effect_owned_around_middleware() {
        let plugin_key: PluginKey = "example.operation".parse().expect("plugin key");
        let mut config = PluginsConfig::default();
        config.list.insert(plugin_key.to_string(), configured(&[]));
        let host = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![StaticPluginRegistration::new(
                plugin_key,
                OperationPlugin::new(),
            )],
            config,
            workspace_root: PathBuf::from("/tmp/agena-plugin-operation-pipeline-test"),
            agena_version: "0.1.0".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .expect("build plugin host");

        assert_eq!(
            host.plugins().len(),
            1,
            "operation plugin did not reach loaded state: inspect={:#?} logs={:#?}",
            host.plugin_inspect("example.operation"),
            host.plugin_logs("example.operation", None, 50)
        );
        let before_middleware = host
            .plugin_inspect("example.operation")
            .expect("inspect before middleware")
            .effects
            .expect("effect scope before middleware");
        assert_eq!(
            before_middleware.lifecycle,
            crate::effect_scope::PluginEffectScopeState::Active,
            "effect scope closed before runtime middleware registration: {before_middleware:#?}"
        );

        host.register_operation_middleware(
            "example.operation",
            10,
            "wrap result",
            |dispatch, next| async move {
                let mut result = next.run(dispatch).await?;
                result.summary = format!("wrapped:{}", result.summary);
                Ok(result)
            },
        )
        .expect("register operation middleware");

        let result = host
            .invoke_plugin_operation_async(
                "example.operation",
                agena_plugin_sdk::PluginOperationInvokeInput {
                    operation_id: "run".to_string(),
                    input: serde_json::json!({}),
                    session_id: None,
                    call_id: None,
                    workspace_root: None,
                    slash: Some("run".to_string()),
                    raw: String::new(),
                },
            )
            .await
            .expect("invoke operation");
        assert_eq!(result.summary, "wrapped:terminal");
        assert!(
            host.plugin_inspect("example.operation")
                .expect("inspect")
                .effects
                .as_ref()
                .is_some_and(|scope| scope.effects.iter().any(|effect| {
                    effect.kind == "event.handler" && effect.label == "wrap result"
                }))
        );
        host.shutdown().await;
    }
}
