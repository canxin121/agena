use super::{
    AgenaCli, AgenaMcpBackend, AppError, Arc, AtomicI64, McpServerArgs, OutputFormat, PathBuf,
    ProviderCapabilitiesOutput, ProviderCommand, ProviderDefaultsSummary, ProviderListArgs,
    ProviderListOutput, ProviderModelsOutput, ProviderSubcommand, ProviderSummary,
    render_serialized,
};
use agena_application::Application;
use agena_domain::ProviderId;
use agena_runtime::bootstrap_application_services;
impl AgenaCli {
    pub(super) async fn session_runtime(
        &self,
    ) -> Result<agena_runtime::RuntimeBootstrapResult, AppError> {
        self.session_runtime_with_workspace(None).await
    }

    pub(super) async fn session_runtime_with_workspace(
        &self,
        workspace: Option<&PathBuf>,
    ) -> Result<agena_runtime::RuntimeBootstrapResult, AppError> {
        bootstrap_application_services(self.runtime_bootstrap_request(workspace)?)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))
    }

    /// Run a short-lived CLI operation against Runtime services while retaining
    /// the bootstrap result's lifecycle handle until the operation finishes.
    pub(super) async fn with_session_runtime_services<T, F, Fut>(
        &self,
        operation: F,
    ) -> Result<T, AppError>
    where
        F: FnOnce(agena_runtime::RuntimeApplicationServices) -> Fut,
        Fut: std::future::Future<Output = Result<T, AppError>>,
    {
        let runtime = bootstrap_application_services(self.runtime_bootstrap_request(None)?)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let result = operation(runtime.application_services()).await;
        runtime.shutdown();
        result
    }

    /// Run a short-lived CLI use case through Application while retaining the
    /// bootstrap lifecycle handle until it completes.
    pub(super) async fn with_application<T, F, Fut>(&self, operation: F) -> Result<T, AppError>
    where
        F: FnOnce(Application) -> Fut,
        Fut: std::future::Future<Output = Result<T, AppError>>,
    {
        let runtime = bootstrap_application_services(self.runtime_bootstrap_request(None)?)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let application =
            Application::from_composed_runtime_services(runtime.application_services())
                .map_err(|error| AppError::Internal(error.to_string()))?;
        let result = operation(application).await;
        runtime.shutdown();
        result
    }

    fn runtime_bootstrap_request(
        &self,
        workspace: Option<&PathBuf>,
    ) -> Result<agena_runtime::RuntimeBootstrapRequest, AppError> {
        Ok(agena_runtime::RuntimeBootstrapRequest {
            workspace_root: workspace.cloned(),
            config_override_expressions: self.overrides.clone(),
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
            initialize_schema: true,
            tracing_reload_handle: None,
        })
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

    pub(super) async fn render_provider_command(
        &self,
        command: ProviderCommand,
    ) -> Result<String, AppError> {
        self.with_session_runtime_services(|services| async move {
            let providers = services.provider_catalog;
            match command
                .command
                .unwrap_or(ProviderSubcommand::List(ProviderListArgs {
                    format: OutputFormat::Json,
                })) {
                ProviderSubcommand::List(args) => {
                    let mut providers = providers
                        .list_providers()
                        .into_iter()
                        .map(|provider| ProviderSummary {
                            defaults: ProviderDefaultsSummary {
                                adapter: provider.defaults.adapter,
                                model: provider.defaults.model,
                            },
                            provider_id: provider.provider_id.to_string(),
                        })
                        .collect::<Vec<_>>();
                    providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                    render_serialized(args.format, &ProviderListOutput { providers })
                }
                ProviderSubcommand::Models(args) => {
                    let provider_id = args
                        .provider_id
                        .parse::<ProviderId>()
                        .map_err(|error| AppError::Config(error.to_string()))?;
                    let models = providers
                        .list_models(&provider_id)
                        .await
                        .map_err(|error| AppError::Config(error.to_string()))?;
                    render_serialized(
                        args.format,
                        &ProviderModelsOutput {
                            provider_id: args.provider_id,
                            models,
                        },
                    )
                }
                ProviderSubcommand::Capabilities(args) => {
                    let model_ref = providers
                        .resolve_model_target(args.target.as_str(), args.model.as_deref())
                        .map_err(|error| AppError::Config(error.to_string()))?;
                    let options = providers
                        .model_execution_options(&model_ref)
                        .map_err(|error| AppError::Config(error.to_string()))?;
                    render_serialized(
                        args.format,
                        &ProviderCapabilitiesOutput {
                            provider_id: model_ref.provider_id.to_string(),
                            model: model_ref.model_id.to_string(),
                            model_ref: model_ref.to_string(),
                            capabilities: options.capabilities,
                            metadata: options.metadata,
                        },
                    )
                }
            }
        })
        .await
    }

    pub(super) async fn mcp_server_backend(
        &self,
        args: McpServerArgs,
    ) -> Result<AgenaMcpBackend, AppError> {
        let runtime = self
            .session_runtime_with_workspace(args.workspace.as_ref())
            .await?;
        let services = runtime.application_services();
        Ok(AgenaMcpBackend {
            runtime,
            tools: services.tools,
            event_publisher: services.event_publisher,
            next_call_id: Arc::new(AtomicI64::new(1)),
        })
    }
}
