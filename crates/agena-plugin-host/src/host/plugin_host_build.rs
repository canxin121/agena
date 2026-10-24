enum BuildCandidate {
    Prepared(Box<PreparedPlugin>),
    Reused(Arc<LoadedPlugin>),
}

/// A successfully rebound transport is still referenced by the predecessor.
/// Dropping an unfinished build must close it explicitly: dropping the new
/// HostHandle alone cannot end that process or update the predecessor status.
#[derive(Default)]
struct ReusedBuildGuard {
    transfers: Vec<(Arc<PluginHost>, Arc<HostHandle>, Arc<LoadedPlugin>)>,
    committed: bool,
}

impl Drop for ReusedBuildGuard {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for (previous, successor, plugin) in std::mem::take(&mut self.transfers) {
            tokio::spawn(async move {
                let plugin_id = plugin.key();
                let mut diagnostic =
                    "plugin host build did not complete after transport handoff".to_string();
                if let Err(error) = crate::loader::close_transport_for_plugin(
                    &plugin_id.to_string(),
                    plugin.transport.as_ref(),
                )
                .await
                {
                    diagnostic.push_str(&format!("; cleanup: {error}"));
                }
                previous
                    .statuses
                    .record_spawn_failure(&plugin_id, &diagnostic);
                previous.logs.append(
                    &plugin_id,
                    "error",
                    "reload",
                    diagnostic,
                    serde_json::Value::Null,
                );
                successor.dispose_plugin_resources(&plugin_id).await;
                previous
                    ._host_handle
                    .dispose_plugin_resources(&plugin_id)
                    .await;
            });
        }
    }
}

impl BuildCandidate {
    fn manifest(&self) -> &crate::sdk::PluginManifest {
        match self {
            Self::Prepared(plugin) => &plugin.manifest,
            Self::Reused(plugin) => &plugin.manifest,
        }
    }
}

fn record_runtime_restart(
    plan: &mut crate::activation::PluginReloadPlan,
    plugin_id: &PluginKey,
    reason: crate::activation::PluginReloadReason,
    dependencies: &[PluginKey],
) {
    let Some(decision) = plan
        .decisions
        .iter_mut()
        .find(|decision| &decision.plugin_id == plugin_id)
    else {
        return;
    };
    if decision.action == crate::activation::PluginReloadAction::Reuse {
        decision.action = crate::activation::PluginReloadAction::Restart;
    }
    if !decision.reasons.contains(&reason) {
        decision.reasons.push(reason);
        decision.reasons.sort();
    }
    decision.triggered_by.extend_from_slice(dependencies);
    decision.triggered_by.sort();
    decision.triggered_by.dedup();
}

impl PluginHost {
    pub async fn new(build: PluginHostBuildConfig) -> Result<Arc<PluginHost>, HostError> {
        let dispatcher = build
            .previous
            .as_ref()
            .map(|host| host._host_handle.callback_dispatcher())
            .unwrap_or_default();
        Self::new_with_callback_dispatcher(build, dispatcher).await
    }

    /// Compose a host against a dispatcher that may already be serving its
    /// first initialization callbacks. Reloads must preserve the lineage.
    pub async fn new_with_callback_dispatcher(
        build: PluginHostBuildConfig,
        dispatcher: crate::PluginCallbackDispatcher,
    ) -> Result<Arc<PluginHost>, HostError> {
        if let Some(previous) = &build.previous
            && !Arc::ptr_eq(&previous._host_handle.callback_routes, &dispatcher.routes)
        {
            return Err(HostError::Config(
                "callback dispatcher must belong to the previous host lineage".into(),
            ));
        }
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
        handle.callback_routes = dispatcher.routes;
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
            if reusable {
                if let Some(previous_plugin) = previous_loaded.get(&plugin_key).cloned()
                    && previous_plugin
                        .transport
                        .can_reuse(&previous_plugin.effect_scope)
                        .await
                {
                    prefetched_manifests.insert(id.clone(), previous_plugin.manifest.clone());
                    candidates.insert(id.clone(), BuildCandidate::Reused(previous_plugin));
                    continue;
                }
                record_runtime_restart(
                    &mut reload_plan,
                    &plugin_key,
                    crate::activation::PluginReloadReason::RuntimeUnavailable,
                    &[],
                );
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
        let mut replaced_instances = BTreeSet::<String>::new();
        let mut reused_build = ReusedBuildGuard::default();

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

            let replaced_dependencies = effective
                .activation
                .requires
                .iter()
                .filter(|dependency| replaced_instances.contains(&dependency.to_string()))
                .cloned()
                .collect::<Vec<_>>();
            if !replaced_dependencies.is_empty() {
                record_runtime_restart(
                    &mut reload_plan,
                    &plugin_key,
                    crate::activation::PluginReloadReason::DependencyEpochChanged,
                    &replaced_dependencies,
                );
            }
            let mut force_prepare = false;
            let plugin = loop {
                let unavailable = if let BuildCandidate::Reused(plugin) = &candidate {
                    !plugin.transport.can_reuse(&plugin.effect_scope).await
                } else {
                    false
                };
                if unavailable || force_prepare {
                    record_runtime_restart(
                        &mut reload_plan,
                        &plugin_key,
                        crate::activation::PluginReloadReason::RuntimeUnavailable,
                        &[],
                    );
                }
                if matches!(candidate, BuildCandidate::Reused(_))
                    && (force_prepare
                        || unavailable
                        || !replaced_dependencies.is_empty()
                        || !service_epoch_unchanged(
                            &id,
                            &service_plan.bindings,
                            &previous_service_bindings,
                            &config.list,
                            &previous_plugins,
                        ))
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
                            host_handle.rollback_failed_plugin(&plugin_key).await;
                            let block = crate::activation::PluginActivationBlock {
                                plugin_id: id.clone(),
                                code: "manifest_changed_during_reload",
                                message: concat!(
                                    "plugin manifest changed while replacing a reuse candidate; ",
                                    "retry reload to resolve a fresh graph"
                                )
                                .to_string(),
                                dependencies: Vec::new(),
                            };
                            activation_blocks.insert(id.clone(), block.clone());
                            record_activation_block(&statuses_shared, &logs_shared, &block);
                            break None;
                        }
                        Err(error) => {
                            let block = crate::activation::PluginActivationBlock {
                                plugin_id: id.clone(),
                                code: "preparation_failed",
                                message: concat!(
                                    "plugin or dependency instance became unavailable for reuse ",
                                    "and could not be prepared again"
                                )
                                .to_string(),
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
                            break None;
                        }
                    }
                }

                // Install immutable defaults before init or live-state handoff.
                // Dynamic registrations may intentionally replace these defaults.
                tool_registry_shared
                    .write()
                    .map_err(|_| HostError::Config("plugin tool registry lock poisoned".into()))?
                    .extend_from_plugin(&plugin_key, &candidate.manifest().tools)
                    .map_err(|message| HostError::Load {
                        plugin: plugin_key.to_string(),
                        message,
                    })?;

                match candidate {
                    BuildCandidate::Reused(reused) => {
                        tracing::info!(
                            target: "agena_plugin_host",
                            plugin = %id,
                            "reusing existing plugin transport (config and dependency epochs unchanged)"
                        );
                        let scope = host_handle.begin_plugin_instance(plugin_key.clone());
                        if let Some(previous_status) = previous
                            .as_ref()
                            .and_then(|host| host.plugin_status_by_key(&plugin_key))
                        {
                            statuses_shared.set(previous_status);
                        }
                        let attach_context =
                            "failed to attach the successor host to a reused plugin transport";
                        let bound = reused
                            .transport
                            .try_rebind_host(
                                Arc::clone(&host_handle),
                                Arc::clone(&scope),
                                Arc::clone(&reused.effect_scope),
                                crate::transport::initialization::PluginInitialization {
                                    host: Arc::downgrade(&host_handle),
                                    plugin_id: plugin_key.clone(),
                                    manifest: reused.manifest.clone(),
                                    agena_version: agena_version.clone(),
                                    workspace_root: workspace_root.clone(),
                                    settings: reused.configured_plugin.settings().clone(),
                                },
                            )
                            .await
                            .map_err(|error| HostError::Load {
                                plugin: id.clone(),
                                message: agena_failure::diagnostic::format_error_chain_with_context(
                                    attach_context,
                                    &error,
                                ),
                            })?;
                        if !bound {
                            statuses_shared.set(crate::status::PluginStatus::initial(
                                &plugin_key,
                                reused.kind,
                            ));
                            force_prepare = true;
                            candidate = BuildCandidate::Reused(reused);
                            continue;
                        }
                        if let Some(previous_host) = &previous {
                            reused_build.transfers.push((
                                Arc::clone(previous_host),
                                Arc::clone(&host_handle),
                                Arc::clone(&reused),
                            ));
                            previous_host
                                .transferred_to_successor
                                .lock()
                                .await
                                .insert(plugin_key.clone());
                        }
                        break Some(Arc::new(reused.rebind_effect_scope(scope)));
                    }
                    BuildCandidate::Prepared(prepared) => {
                        let transport = prepared.transport();
                        match activate_entry(
                            *prepared,
                            &host_handle,
                            &agena_version,
                            &workspace_root,
                        )
                        .await
                        {
                            Ok(plugin) => break Some(Arc::new(plugin)),
                            Err(error) => {
                                if let Err(close_error) = crate::loader::close_transport_for_plugin(
                                    &id,
                                    transport.as_ref(),
                                )
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
                                break None;
                            }
                        }
                    }
                }
            };
            let Some(plugin) = plugin else {
                continue;
            };
            let registration = async {
                host_handle.own_manifest_resources(&plugin.key(), &plugin.manifest)?;
                host_handle.set_plugin_hook_catalog(hook_registration_for_plugin(&plugin))?;
                host_handle
                    .register_plugin_transport(plugin.key(), plugin.transport())
                    .await
            }
            .await;
            if let Err(error) = registration {
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
                host_handle
                    .rollback_failed_plugin_for_scope(&plugin.key(), &plugin.effect_scope)
                    .await;
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
            by_id.insert(plugin.key(), Arc::clone(&plugin));
            if !previous_loaded
                .get(&plugin.key())
                .is_some_and(|previous| Arc::ptr_eq(&previous.transport, &plugin.transport))
            {
                replaced_instances.insert(id.clone());
            }
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

        let command_registry = host_handle.command_registry();
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
        let host = Arc::new(PluginHost {
            plugins: loaded,
            plugins_by_id: by_id,
            tool_registry: tool_registry_shared,
            command_registry,
            command_pipeline: Arc::new(crate::event_pipeline::PluginAroundPipeline::new()),
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
        });
        reused_build.committed = true;
        Ok(host)
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
