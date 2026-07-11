use super::{
    Arc, ConfigError, ModelCatalogSnapshot, PathBuf, PluginHost, PluginHostBuildConfig,
    ProcessEnvironment, ProviderRegistry,
};

impl crate::config::ConfigResolution {
    pub async fn build_provider_registry_with_plugins(
        &self,
        plugins: &PluginHost,
    ) -> Result<ProviderRegistry, ConfigError> {
        self.build_provider_registry_with_plugins_and_catalog(plugins, None)
            .await
    }

    pub async fn build_provider_registry_with_plugins_and_catalog(
        &self,
        plugins: &PluginHost,
        catalog: Option<&ModelCatalogSnapshot>,
    ) -> Result<ProviderRegistry, ConfigError> {
        let mut registry = self
            .config
            .build_provider_registry_with_catalog_and_env_and_config_path(
                catalog,
                &ProcessEnvironment,
                Some(self.meta.config_path.as_path()),
            )?;
        if plugins.is_empty() {
            return Ok(registry);
        }

        let current = registry
            .provider_ids()
            .into_iter()
            .map(|id| crate::plugin::ProviderDescriptor {
                display_name: id.clone(),
                id,
                models: Vec::new(),
                endpoint: None,
                kind: crate::plugin::ProviderKind::Custom,
            })
            .collect();
        let patch = plugins
            .dispatch_provider_list(crate::plugin::ProviderListInput { current })
            .await
            .map_err(|err| ConfigError::Validation(format!("plugin provider.list: {err}")))?;
        for provider_id in patch.remove {
            registry.remove(provider_id.as_ref());
        }
        for descriptor in patch.add {
            registry.register_plugin_provider(descriptor)?;
        }
        Ok(registry)
    }

    pub async fn build_plugin_host(&self) -> Result<Arc<PluginHost>, ConfigError> {
        self.build_plugin_host_with_previous_and_mcp(None, None, None)
            .await
    }

    /// Hot-reload-aware build: when a previous plugin host (and its config)
    /// is available, transports for byte-identical entries are reused, so
    /// stdio subprocesses and HTTP plugins survive a config reload that
    /// didn't touch them.
    pub async fn build_plugin_host_with_previous(
        &self,
        previous_host: Option<Arc<PluginHost>>,
        previous_config: Option<&agena_plugin_host::PluginsConfig>,
    ) -> Result<Arc<PluginHost>, ConfigError> {
        self.build_plugin_host_with_previous_and_mcp(previous_host, previous_config, None)
            .await
    }

    pub async fn build_plugin_host_with_previous_and_mcp(
        &self,
        previous_host: Option<Arc<PluginHost>>,
        previous_config: Option<&agena_plugin_host::PluginsConfig>,
        mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
    ) -> Result<Arc<PluginHost>, ConfigError> {
        let workspace_root = self
            .meta
            .config_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let agena_version = env!("CARGO_PKG_VERSION").to_string();
        let plugin_config = self.config.plugins.clone();
        let mcp_manager = match mcp_manager {
            Some(manager) => Some(manager),
            None => self.build_mcp_manager_from_plugin_config().await?,
        };
        let static_plugins = crate::plugins::sources::static_plugin_registrations(mcp_manager);
        let previous_plugins = previous_config
            .map(PluginHostBuildConfig::previous_plugins)
            .unwrap_or_default();
        let build_config = PluginHostBuildConfig {
            static_plugins,
            config: plugin_config,
            workspace_root,
            agena_version,
            callback_base_url: None,
            host_client: None,
            previous: previous_host,
            previous_plugins,
        };
        PluginHost::new(build_config)
            .await
            .map_err(|e| ConfigError::Validation(format!("plugin host: {e}")))
    }

    async fn build_mcp_manager_from_plugin_config(
        &self,
    ) -> Result<Option<Arc<agena_mcp_client::McpConnectionManager>>, ConfigError> {
        let resolved_plugins =
            crate::plugins::sources::resolve_plugin_config(self.config.plugins.clone());
        if !crate::plugins::provided::mcp::static_bridge_enabled(&resolved_plugins) {
            return Ok(None);
        }
        let config = crate::plugins::provided::mcp::config_from_plugins(&resolved_plugins)
            .map_err(ConfigError::Validation)?;
        Ok(Some(
            crate::plugins::provided::mcp::build_manager(&config).await,
        ))
    }
}
