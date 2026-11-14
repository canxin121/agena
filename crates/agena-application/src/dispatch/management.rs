//! Application execution of the shared control-plane contract.
mod marketplace;
mod plugins;
mod providers;
mod settings;
mod usage;

use crate::{Application, ApplicationError};
use agena_api::management::*;
use serde::Serialize;
use serde_json::Value;

impl Application {
    pub(crate) async fn dispatch_management_query(
        &self,
        query: ManagementQuery,
    ) -> Result<Value, ApplicationError> {
        match query {
            ManagementQuery::Settings(query) => self.query_settings(query).await,
            ManagementQuery::Providers(query) => self.query_provider_management(query).await,
            ManagementQuery::Auth(query) => self.query_auth_management(query).await,
            ManagementQuery::ModelCatalog(query) => match query {
                ModelCatalogQuery::List {
                    q,
                    origin,
                    offset,
                    limit,
                } => wire(self.list_model_catalog_with_origin(
                    &q,
                    origin.as_deref(),
                    offset,
                    agena_api::pagination::normalize_resource_limit(limit.map(|value| value as u64))
                        as usize,
                )),
                ModelCatalogQuery::Lookup { model_ids } => {
                    Ok(serde_json::json!({ "items": self.lookup_model_catalog_models(&model_ids) }))
                }
                ModelCatalogQuery::Match { model_ids } => {
                    Ok(serde_json::json!({ "items": self.match_model_catalog_models(&model_ids) }))
                }
            },
            ManagementQuery::Mcp(query) => match query {
                McpQuery::List => wire(self.mcp_connections().await?),
                McpQuery::Get { id } => wire(self.mcp_connection(id).await?),
                McpQuery::PreviewImport(request) => wire(self.preview_mcp_import(request).await?),
                McpQuery::Capabilities { id, kind, cursor } => {
                    wire(self.mcp_connection_capabilities(id, kind, cursor).await?)
                }
                McpQuery::ReadResource { id, request } => {
                    self.read_mcp_connection_resource(id, request).await
                }
                McpQuery::GetPrompt { id, request } => {
                    self.get_mcp_connection_prompt(id, request).await
                }
            },
            ManagementQuery::Skills(query) => match query {
                SkillsQuery::List { workspace_id } => wire(self.list_skills(workspace_id).await?),
                SkillsQuery::Get { workspace_id, id } => {
                    wire(self.get_skill(workspace_id, id).await?)
                }
                SkillsQuery::Resource {
                    workspace_id,
                    id,
                    path,
                } => wire(self.get_skill_resource(workspace_id, id, path).await?),
                SkillsQuery::Validate {
                    workspace_id,
                    request,
                } => wire(self.validate_skill(workspace_id, request).await?),
                SkillsQuery::PreviewInstall {
                    workspace_id,
                    request,
                } => wire(self.preview_skill_install(workspace_id, request).await?),
            },
            ManagementQuery::Plugins(query) => self.query_plugin_management(query).await,
            ManagementQuery::Marketplace(query) => match query {
                MarketplaceQuery::Search(request) => self.search_marketplace_plugins(request).await,
                MarketplaceQuery::Installed => self.list_marketplace_installed_plugins().await,
                MarketplaceQuery::Outdated => self.list_marketplace_outdated_plugins().await,
            },
            ManagementQuery::RuntimeTasks => Ok(
                serde_json::json!({ "items": self.runtime_control().background_tasks()
                .into_iter().map(crate::dto::runtime_background_task_resource).collect::<Vec<_>>() }),
            ),
            ManagementQuery::RuntimeMetrics => wire(self.runtime_metrics()),
            ManagementQuery::RuntimeDiagnostics => wire(self.runtime_diagnostics().await),
            ManagementQuery::Usage(query) => self.query_usage_stats(query).await,
            ManagementQuery::OperatorTools => wire(self.list_operator_tools().await),
        }
    }

    pub(crate) async fn dispatch_management_command(
        &self,
        command: ManagementCommand,
    ) -> Result<Value, ApplicationError> {
        match command {
            ManagementCommand::Settings(command) => self.command_settings(command).await,
            ManagementCommand::Providers(command) => {
                self.command_provider_management(command).await
            }
            ManagementCommand::Auth(command) => self.command_auth_management(command).await,
            ManagementCommand::ModelCatalogRefresh => wire(self.refresh_model_catalog()?),
            ManagementCommand::Mcp(command) => match command {
                McpCommand::Create(request) => wire(self.create_mcp_connection(request).await?),
                McpCommand::Update { id, request } => {
                    wire(self.update_mcp_connection(id, request).await?)
                }
                McpCommand::Remove {
                    id,
                    expected_revision,
                } => wire(self.remove_mcp_connection(id, expected_revision).await?),
                McpCommand::SetEnabled { id, request } => {
                    wire(self.set_mcp_connection_enabled(id, request).await?)
                }
                McpCommand::SetBridgeEnabled(request) => {
                    wire(self.set_mcp_bridge_enabled(request).await?)
                }
                McpCommand::Import(request) => wire(self.import_mcp_connections(request).await?),
                McpCommand::Reconnect { id } => wire(self.reconnect_mcp_connection(id).await?),
                McpCommand::Test { id } => wire(self.test_mcp_connection(id).await?),
                McpCommand::SetCredential { id, token } => {
                    wire(self.set_mcp_connection_credential(id, Some(token)).await?)
                }
                McpCommand::DeleteCredential { id } => {
                    wire(self.set_mcp_connection_credential(id, None).await?)
                }
                McpCommand::StartAuth { id, request } => {
                    wire(self.start_mcp_connection_auth(id, request).await?)
                }
                McpCommand::CompleteAuth { id, request } => {
                    wire(self.complete_mcp_connection_auth(id, request).await?)
                }
                McpCommand::CancelAuth { id, flow_id } => {
                    wire(self.clear_mcp_connection_auth(id, Some(flow_id)).await?)
                }
                McpCommand::Logout { id } => wire(self.clear_mcp_connection_auth(id, None).await?),
            },
            ManagementCommand::Skills(command) => match command {
                SkillsCommand::Create {
                    workspace_id,
                    request,
                } => wire(self.create_skill(workspace_id, request).await?),
                SkillsCommand::Update {
                    workspace_id,
                    id,
                    request,
                } => wire(self.update_skill(workspace_id, id, request).await?),
                SkillsCommand::Delete {
                    workspace_id,
                    id,
                    expected_revision,
                } => wire(
                    self.delete_skill(workspace_id, id, expected_revision)
                        .await?,
                ),
                SkillsCommand::SetEnabled {
                    workspace_id,
                    id,
                    request,
                } => wire(self.set_skill_enabled(workspace_id, id, request).await?),
                SkillsCommand::WriteResource {
                    workspace_id,
                    id,
                    request,
                } => wire(self.write_skill_resource(workspace_id, id, request).await?),
                SkillsCommand::DeleteResource {
                    workspace_id,
                    id,
                    path,
                    expected_revision,
                } => wire(
                    self.delete_skill_resource(workspace_id, id, path, expected_revision)
                        .await?,
                ),
                SkillsCommand::Install {
                    workspace_id,
                    request,
                } => wire(self.install_skills(workspace_id, request).await?),
            },
            ManagementCommand::Plugins(command) => self.command_plugin_management(command).await,
            ManagementCommand::Marketplace(command) => match command {
                MarketplaceCommand::Sync(request) => self.sync_marketplace_registry(request).await,
                MarketplaceCommand::Install(request) => {
                    self.install_marketplace_plugin(request).await
                }
                MarketplaceCommand::Uninstall(request) => {
                    self.uninstall_marketplace_plugin(request).await
                }
                MarketplaceCommand::Upgrade(request) => {
                    self.upgrade_marketplace_plugins(request).await
                }
            },
            ManagementCommand::ReloadRuntime => wire(runtime_background_task_start_response(
                self.runtime_control()
                    .start_runtime_reload_task(
                        agena_runtime::RuntimeReloadCause::Manual,
                        agena_runtime::RuntimeBackgroundTaskOrigin::User,
                    )
                    .map_err(ApplicationError::from)?,
            )),
            ManagementCommand::CancelRuntimeTask { task_id } => {
                wire(agena_api::resource::RuntimeBackgroundTaskCancelResponse {
                    task: crate::dto::runtime_background_task_resource(
                        self.runtime_control()
                            .cancel_background_task(task_id.trim())
                            .map_err(ApplicationError::from)?,
                    ),
                })
            }
            ManagementCommand::InvokeOperatorTool {
                workspace_id,
                tool,
                input,
            } => wire(
                self.execute_operator_tool(workspace_id, &tool, input)
                    .await?,
            ),
            ManagementCommand::PermissionRulesBatch(request) => {
                wire(self.service().batch_permission_rules(request).await?)
            }
        }
    }

    /// Allocate invocation IDs in the common application boundary so HTTP and
    /// plugin-issued operator calls cannot collide.
    pub async fn execute_operator_tool(
        &self,
        workspace_id: i64,
        tool: &str,
        input: Option<Value>,
    ) -> Result<agena_tool::ToolExecutionSummary, ApplicationError> {
        static NEXT_CALL_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
        let call_id = NEXT_CALL_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.invoke_operator_tool(workspace_id, tool, input, call_id)
            .await
    }
}

fn wire<T: Serialize>(value: T) -> Result<Value, ApplicationError> {
    serde_json::to_value(value).map_err(|error| ApplicationError::internal_error(&error))
}

fn runtime_background_task_start_response(
    start: agena_runtime::RuntimeBackgroundTaskStart,
) -> agena_api::resource::RuntimeBackgroundTaskStartResponse {
    agena_api::resource::RuntimeBackgroundTaskStartResponse {
        started: start.started,
        task: crate::dto::runtime_background_task_resource(start.task),
    }
}
