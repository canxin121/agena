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
            callback_base_url.clone(),
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
            Box::new(|k: &str| std::env::var(k).ok());

        let mut static_registry: HashMap<PluginKey, StaticRegistration> = static_plugins
            .into_iter()
            .map(|entry| (entry.key, entry.registration))
            .collect();
        let mut loaded: Vec<Arc<LoadedPlugin>> = Vec::new();
        let mut by_id: HashMap<PluginKey, Arc<LoadedPlugin>> = HashMap::new();

        // Sort configured plugins by id for deterministic load order.
        let mut configured_plugins: Vec<(String, ConfiguredPlugin)> =
            config.list.into_iter().collect();
        configured_plugins.sort_by(|a, b| a.0.cmp(&b.0));

        // Build a quick lookup of previous LoadedPlugin by id for reuse.
        let previous_loaded: HashMap<PluginKey, Arc<LoadedPlugin>> = previous
            .as_ref()
            .map(|p| {
                p.plugins
                    .iter()
                    .map(|lp| (lp.key(), Arc::clone(lp)))
                    .collect()
            })
            .unwrap_or_default();

        for (idx, (id, configured_plugin)) in configured_plugins.into_iter().enumerate() {
            let plugin_key: PluginKey = id.parse().map_err(|err| {
                HostError::Config(format!("invalid plugin id `{id}` in plugins.list: {err}"))
            })?;
            statuses_shared.set(crate::status::PluginStatus::initial(
                &plugin_key,
                configured_plugin.kind_str(),
            ));
            if configured_plugin.disabled() {
                statuses_shared.record_stopped(&plugin_key);
                tracing::info!(
                    target: "agena_plugin_host",
                    plugin = %id,
                    kind = configured_plugin.kind_str(),
                    "plugin disabled in config; skipping load"
                );
                continue;
            }
            plugin_indices
                .write()
                .map_err(|_| HostError::Config("plugin index registry lock poisoned".into()))?
                .insert(plugin_key.clone(), idx);
            // Hot-reload: if a previous host had this id with a byte-identical
            // configured plugin, reuse the transport (no respawn) — except for
            // in-proc Static plugins. A static plugin instance binds its host
            // (ScopedHostClient -> HostHandle) during `meta/init`; reusing the
            // transport after a reload keeps that binding pointing at the
            // detached previous handle, so every display/theme/notification
            // write silently misses the live host. Static plugins are cheap to
            // rebuild and re-init against the current handle.
            if let Some(previous_plugin) = previous_plugins.get(&id)
                && previous_plugin == &configured_plugin
                && !matches!(&configured_plugin.package, PluginPackage::Static { .. })
                && let Some(reused) = previous_loaded.get(&plugin_key).cloned()
            {
                tracing::info!(
                    target: "agena_plugin_host",
                    plugin = %id,
                    "reusing existing plugin transport (config unchanged)"
                );
                if let Some(prev_host) = &previous {
                    prev_host
                        .transferred_to_successor
                        .lock()
                        .await
                        .insert(plugin_key.clone());
                }
                reused
                    .transport
                    .attach_host(host_handle.scoped_host_client(reused.key().to_string()))
                    .await
                    .map_err(|e| HostError::Load {
                        plugin: reused.key().to_string(),
                        message: e.to_string(),
                    })?;
                tool_registry_shared
                    .write()
                    .map_err(|_| HostError::Config("plugin tool registry lock poisoned".into()))?
                    .extend_from_plugin(
                        &reused.key(),
                        &reused.manifest.tools,
                    )
                    .map_err(|message| HostError::Load {
                        plugin: reused.key().to_string(),
                        message,
                    })?;
                plugin_names
                    .write()
                    .map_err(|_| HostError::Config("plugin name registry lock poisoned".into()))?
                    .insert(reused.key(), reused.manifest.name.clone());
                host_handle.set_plugin_hook_catalog(hook_registration_for_plugin(&reused));
                if let Some(previous_status) = previous
                    .as_ref()
                    .and_then(|previous| previous.plugin_status_by_key(&reused.key()))
                {
                    statuses_shared.set(previous_status);
                }
                by_id.insert(reused.key(), Arc::clone(&reused));
                host_handle
                    .register_plugin_transport(reused.key(), reused.transport())
                    .await;
                loaded.push(reused);
                continue;
            }
            match load_entry(
                &id,
                &configured_plugin,
                &mut static_registry,
                Arc::clone(&host_handle),
                &agena_version,
                &workspace_root,
                &env_lookup,
                &config.host.trusted_keys,
            )
            .await
            {
                Ok(plugin) => {
                    let plugin = Arc::new(plugin);
                    tool_registry_shared
                        .write()
                        .map_err(|_| {
                            HostError::Config("plugin tool registry lock poisoned".into())
                        })?
                        .extend_from_plugin(
                            &plugin.key(),
                            &plugin.manifest.tools,
                        )
                        .map_err(|message| HostError::Load {
                            plugin: plugin.key().to_string(),
                            message,
                        })?;
                    plugin_names
                        .write()
                        .map_err(|_| {
                            HostError::Config("plugin name registry lock poisoned".into())
                        })?
                        .insert(plugin.key(), plugin.manifest.name.clone());
                    host_handle.set_plugin_hook_catalog(hook_registration_for_plugin(&plugin));
                    let status_kind = plugin.kind;
                    let initial = crate::status::PluginStatus::initial(&plugin.key(), status_kind);
                    statuses_shared.set(initial);
                    by_id.insert(plugin.key(), plugin.clone());
                    host_handle
                        .register_plugin_transport(plugin.key(), plugin.transport())
                        .await;
                    loaded.push(plugin);
                }
                Err(err) => {
                    host_handle.rollback_failed_plugin(&plugin_key).await;
                    let message = err.to_string();
                    statuses_shared.record_spawn_failure(&plugin_key, message.clone());
                    logs_shared.append(
                        &plugin_key,
                        "error",
                        "host",
                        format!("failed to load plugin: {message}"),
                        serde_json::Value::Null,
                    );
                    tracing::warn!(
                        target: "agena_plugin_host",
                        plugin = %id,
                        "failed to load plugin: {err}"
                    );
                }
            }
        }

        Ok(Arc::new(PluginHost {
            plugins: loaded,
            plugins_by_id: by_id,
            tool_registry: tool_registry_shared,
            statuses: statuses_shared,
            logs: logs_shared,
            timeouts: config.host.timeouts,
            runtime: None,
            runtime_handle: tokio::runtime::Handle::try_current().ok(),
            _host_handle: host_handle,
            transferred_to_successor: tokio::sync::Mutex::new(Default::default()),
            hook_runs: Arc::new(std::sync::Mutex::new(Vec::new())),
        }))
    }
}
use super::{
    Arc, ConfiguredPlugin, HashMap, HostError, HostHandle, LoadedPlugin, NoopHostClient,
    PluginHost, PluginHostBuildConfig, PluginKey, PluginLogStore, PluginPackage,
    PluginToolRegistry, RwLock, StaticRegistration, hook_registration_for_plugin, load_entry,
};
