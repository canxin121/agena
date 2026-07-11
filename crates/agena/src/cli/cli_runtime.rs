use super::{
    AgenaCli, AgenaMcpBackend, AgenaRuntime, Agent, AgentsCommand, AgentsListArgs,
    AgentsListOutput, AgentsSubcommand, AppError, Arc, AtomicI64, AuthManager, ConfigEnvironment,
    ConfigLoader, ConfigOutputFormat, LoadConfigRequest, McpServerArgs, MemoryStore, PathBuf,
    PermissionPolicy, ProviderCapabilitiesOutput, ProviderCommand, ProviderConfigCredentialStore,
    ProviderDefaultsSummary, ProviderListArgs, ProviderListOutput, ProviderModelsOutput,
    ProviderSubcommand, ProviderSummary, StorageConfig, ToolExecutor, ToolPermissionPolicy,
    render_serialized,
};

impl AgenaCli {
    pub(super) async fn session_runtime(&self) -> Result<AgenaRuntime, AppError> {
        self.session_runtime_with_workspace(None).await
    }

    pub(super) async fn session_runtime_with_workspace(
        &self,
        workspace: Option<&PathBuf>,
    ) -> Result<AgenaRuntime, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let mut load_request = self.load_request();
        if let Some(workspace) = workspace {
            load_request.workspace_root = Some(workspace.clone());
        }
        AgenaRuntime::new(crate::runtime::AgenaRuntimeConfig {
            load_request,
            workspace_root: workspace.cloned(),
            database_connection: None,
            database_url: Some(database_url),
            auto_migrate: true,
            tracing_reload_handle: None,
        })
        .await
    }

    pub(super) fn memory_store_for_workspace(
        &self,
        workspace: Option<&PathBuf>,
    ) -> Result<MemoryStore, AppError> {
        Ok(MemoryStore::for_workspace(
            self.resolve_workspace_root(workspace)?.as_path(),
        ))
    }

    pub(super) fn resolve_workspace_root(
        &self,
        workspace: Option<&PathBuf>,
    ) -> Result<PathBuf, AppError> {
        workspace
            .cloned()
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)
            .map_err(AppError::from)
    }

    pub(super) fn auth_manager<E>(
        &self,
        loader: &ConfigLoader<E>,
    ) -> Result<AuthManager<ProviderConfigCredentialStore>, AppError>
    where
        E: ConfigEnvironment,
    {
        let resolution = loader.load(&self.load_request())?;
        Ok(AuthManager::new(ProviderConfigCredentialStore::new(
            resolution.meta.config_path,
        )))
    }

    pub(super) async fn render_provider_command<E>(
        &self,
        loader: &ConfigLoader<E>,
        command: ProviderCommand,
    ) -> Result<String, AppError>
    where
        E: ConfigEnvironment,
    {
        let resolution = loader.load(&self.load_request())?;
        let registry = resolution
            .config
            .build_provider_registry_with_env(loader.environment())?;

        match command
            .command
            .unwrap_or(ProviderSubcommand::List(ProviderListArgs {
                format: ConfigOutputFormat::Json,
            })) {
            ProviderSubcommand::List(args) => {
                let mut providers = registry
                    .provider_ids()
                    .into_iter()
                    .filter_map(|provider_id| {
                        registry
                            .get(provider_id.as_str())
                            .map(|provider| ProviderSummary {
                                defaults: ProviderDefaultsSummary {
                                    adapter: provider.default_adapter().map(ToString::to_string),
                                    model: provider.default_model().to_string(),
                                },
                                provider_id,
                            })
                    })
                    .collect::<Vec<_>>();
                providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                render_serialized(args.format, &ProviderListOutput { providers })
            }
            ProviderSubcommand::Models(args) => {
                let models = registry.list_models(args.provider_id.as_str()).await?;
                render_serialized(
                    args.format,
                    &ProviderModelsOutput {
                        provider_id: args.provider_id,
                        models,
                    },
                )
            }
            ProviderSubcommand::Capabilities(args) => {
                let model_ref =
                    registry.resolve_model_target(args.target.as_str(), args.model.as_deref())?;
                let capabilities = registry.model_capabilities(&model_ref)?;
                let metadata = registry.model_metadata(&model_ref)?;
                render_serialized(
                    args.format,
                    &ProviderCapabilitiesOutput {
                        provider_id: model_ref.provider_id.to_string(),
                        model: model_ref.model_id.to_string(),
                        model_ref: model_ref.to_string(),
                        capabilities,
                        metadata,
                    },
                )
            }
        }
    }

    pub(super) async fn render_agents_command(
        &self,
        command: AgentsCommand,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let snapshot = runtime.current_snapshot();
        let resolution = snapshot.config_resolution();
        let mut agents = snapshot.agents().list_descriptors();
        agents.sort_by(|left, right| left.name.cmp(&right.name));
        let default_agent = resolution
            .config
            .default_agent
            .clone()
            .filter(|name| agents.iter().any(|entry| entry.name == *name))
            .or_else(|| agents.iter().map(|entry| entry.name.clone()).next())
            .unwrap_or_else(|| "none".to_string());
        let total_count = agents.len();

        match command
            .command
            .unwrap_or(AgentsSubcommand::List(AgentsListArgs {
                format: ConfigOutputFormat::Json,
            })) {
            AgentsSubcommand::List(args) => render_serialized(
                args.format,
                &AgentsListOutput {
                    default_agent,
                    total_count,
                    agents,
                },
            ),
        }
    }

    pub(super) async fn mcp_server_backend(
        &self,
        args: McpServerArgs,
    ) -> Result<AgenaMcpBackend, AppError> {
        let runtime = self
            .session_runtime_with_workspace(args.workspace.as_ref())
            .await?;
        let snapshot = runtime.current_snapshot();
        let plugins = snapshot.plugin_manager();
        let session_manager = runtime.session_manager();
        let executor = session_manager.as_ref().map_or_else(
            || {
                let agent = Agent::new(
                    "mcp-server",
                    PermissionPolicy::allow_all(),
                    ToolPermissionPolicy::allow_all(),
                );
                ToolExecutor::new(
                    runtime.workspace_root().to_path_buf(),
                    agent,
                    crate::agents::SubagentRegistry::default(),
                    Arc::clone(&plugins),
                    None,
                    None,
                    None,
                    crate::plugin::ToolPresentationConfig::default(),
                )
            },
            |manager| manager.tool_executor(),
        );
        Ok(AgenaMcpBackend {
            executor,
            session_manager,
            workspace_root: runtime.workspace_root().to_path_buf(),
            next_call_id: Arc::new(AtomicI64::new(1)),
        })
    }

    pub fn load_request(&self) -> LoadConfigRequest {
        LoadConfigRequest {
            overrides: self.overrides.clone(),
            workspace_root: None,
        }
    }
}
