use agena_application::dto::{
    RuntimeAgentProfileResource, RuntimeAgentResource, TuiPreferencesResource,
};
use anyhow::{Context, anyhow};

impl Backend {
    /// Render the terminal diagnostic summary from the Runtime-owned status
    /// projection through Application rather than traversing Runtime status.
    pub async fn runtime_snapshot_summary(&self) -> Result<String> {
        let status = self.application.runtime_snapshot_summary().await;
        Ok(format!(
            "generation {} · loaded {} · {} providers · {} plugins",
            status.generation,
            status.loaded_at.to_rfc3339(),
            status.provider_count,
            status.plugin_count,
        ))
    }

    pub fn new(
        services: agena_runtime::RuntimeApplicationServices,
        workspace_root: PathBuf,
    ) -> Result<Self> {
        Ok(Self {
            application: Application::from_composed_runtime_services(services)
                .map_err(|error| anyhow!(error.to_string()))?,
            workspace_root,
            file_index: Arc::new(OnceLock::new()),
        })
    }

    pub async fn list_workspace_sessions(&self, roots_only: bool) -> Result<Vec<SessionResource>> {
        let workspace_id = self.current_workspace_id().await?;
        self.list_sessions_query(ListSessionsParams {
            cursor: None,
            limit: Some(200),
            workspace_id: Some(workspace_id),
            parent_id: None,
            roots: roots_only,
            search: None,
        })
        .await
        .context("failed to list workspace sessions")
    }

    pub async fn list_workspace_sessions_page(
        &self,
        roots_only: bool,
        search: Option<&str>,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<PaginatedResponse<SessionResource>> {
        let workspace_id = self.current_workspace_id().await?;
        match dispatch::dispatch_query(
            &self.application,
            Query::ListSessions(ListSessionsParams {
                cursor,
                limit: Some(limit),
                workspace_id: Some(workspace_id),
                parent_id: None,
                roots: roots_only,
                search: search.map(str::to_string),
            }),
        )
        .await
        .map_err(api_error)?
        {
            QueryResult::Sessions(page) => Ok(page),
            other => Err(anyhow!("unexpected query result: {:?}", other)),
        }
        .context("failed to list workspace sessions page")
    }

    pub async fn create_session(
        &self,
        title: String,
        parent_id: Option<i64>,
    ) -> Result<SessionResource> {
        let workspace = self
            .resolve_workspace_resource(true)
            .await
            .context("failed to resolve workspace for terminal UI")?;

        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::CreateSession(CreateSessionParams {
                workspace_id: workspace.id,
                title,
                parent_id,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Session(session) => Ok(session),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to create session")
    }

    pub async fn rename_session(&self, session_id: i64, title: String) -> Result<SessionResource> {
        let existing = self
            .get_session(session_id)
            .await
            .context("failed to load session before rename")?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;

        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::UpdateSession(UpdateSessionParams {
                session_id,
                title,
                expected_version: Some(existing.version),
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Session(session) => Ok(session),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to rename session")
    }

    pub fn list_providers(&self) -> Vec<ProviderSummaryResource> {
        self.application
            .provider_catalog()
            .list_providers()
            .into_iter()
            .map(|provider| provider_summary_resource_from_catalog(provider, false))
            .collect()
    }

    pub fn list_agent_names(&self) -> Vec<String> {
        self.application
            .agent_statuses()
            .into_iter()
            .map(|agent| agent.name)
            .collect()
    }

    pub fn list_agent_descriptors(&self) -> Vec<RuntimeAgentResource> {
        self.application.agent_statuses()
    }

    pub fn get_agent_profile(&self, name: &str) -> Option<RuntimeAgentProfileResource> {
        self.application.agent_profile(name)
    }

    pub fn config_has_agent(&self, name: &str) -> bool {
        self.config_agent_names().contains(name.trim())
    }

    pub fn config_agent_names(&self) -> HashSet<String> {
        self.application.config_agent_names()
    }

    pub fn default_agent_name(&self) -> Option<String> {
        self.application.default_agent_name()
    }

    pub fn list_aws_profile_names(&self) -> Vec<String> {
        let credentials_path = env::var("AWS_SHARED_CREDENTIALS_FILE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join(".aws/credentials"))
            });
        let config_path = env::var("AWS_CONFIG_FILE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join(".aws/config"))
            });
        let mut profiles = std::collections::BTreeSet::new();
        for path in [credentials_path, config_path].into_iter().flatten() {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            profiles.extend(parse_aws_profile_names(text.as_str()));
        }
        profiles.into_iter().collect()
    }

    pub fn list_configured_providers(&self) -> Vec<ProviderSummaryResource> {
        self.application
            .provider_catalog()
            .list_providers()
            .into_iter()
            .filter(|provider| provider.provider_native_tools.is_some())
            .map(|provider| provider_summary_resource_from_catalog(provider, true))
            .collect()
    }

    pub fn config_path(&self) -> PathBuf {
        self.application
            .config_path()
            .expect("Application configuration projection must provide its config path")
    }

    pub fn config_json_sources(&self) -> Result<ConfigJsonSources> {
        self.application
            .config_json_sources()
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to read Application configuration-source projection")
    }

    pub fn ui_configuration(&self) -> TuiPreferencesResource {
        self.application
            .tui_preferences()
            .expect("Application configuration projection must provide UI preferences")
    }

    pub async fn set_config_setting(
        &self,
        path: &str,
        value: JsonValue,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse> {
        if let Some((plugin_id, config_segments)) = plugin_config_setting_target(path)? {
            return self
                .set_plugin_config_setting(plugin_id.as_str(), config_segments.as_slice(), value)
                .await;
        }
        self.set_config_setting_direct(path, value).await
    }

    pub(super) async fn set_config_setting_direct(
        &self,
        path: &str,
        value: JsonValue,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse> {
        let response = self
            .application
            .runtime_config_settings()
            .set_file_setting(agena_runtime::ConfigSettingsSetInput {
                path: path.trim().to_owned(),
                value,
                options: agena_runtime::ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to set config setting")?;

        if response.reload_required {
            self.application
                .runtime_control()
                .reload()
                .await
                .context("failed to reload runtime after config change")?;
        }
        Ok(response)
    }

    pub async fn reload_runtime(&self) -> Result<()> {
        self.application
            .runtime_control()
            .reload()
            .await
            .context("failed to reload runtime after agent source change")?;
        Ok(())
    }

    pub async fn refresh_provider_client_versions(
        &self,
    ) -> Result<agena_provider::ProviderClientVersions> {
        self.application
            .refresh_provider_client_versions()
            .await
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to refresh provider client versions")
    }

    pub async fn delete_config_setting(
        &self,
        path: &str,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse> {
        if let Some((plugin_id, config_segments)) = plugin_config_setting_target(path)? {
            return self
                .delete_plugin_config_setting(plugin_id.as_str(), config_segments.as_slice())
                .await;
        }
        let response = self
            .application
            .runtime_config_settings()
            .delete_file_setting(agena_runtime::ConfigSettingsDeleteInput {
                path: path.trim().to_owned(),
                options: agena_runtime::ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to delete config setting")?;

        if response.reload_required {
            self.application
                .runtime_control()
                .reload()
                .await
                .context("failed to reload runtime after config change")?;
        }
        Ok(response)
    }

    pub async fn set_workspace_config_setting(
        &self,
        path: &str,
        value: JsonValue,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse> {
        let response = self
            .application
            .runtime_config_settings()
            .set_project_file_setting(agena_runtime::ConfigSettingsSetInput {
                path: path.trim().to_owned(),
                value,
                options: agena_runtime::ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to set workspace config setting")?;

        if response.reload_required {
            self.application
                .runtime_control()
                .reload()
                .await
                .context("failed to reload runtime after workspace config change")?;
        }
        Ok(response)
    }

    pub async fn delete_workspace_config_setting(
        &self,
        path: &str,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse> {
        let response = self
            .application
            .runtime_config_settings()
            .delete_project_file_setting(agena_runtime::ConfigSettingsDeleteInput {
                path: path.trim().to_owned(),
                options: agena_runtime::ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to delete workspace config setting")?;

        if response.reload_required {
            self.application
                .runtime_control()
                .reload()
                .await
                .context("failed to reload runtime after workspace config change")?;
        }
        Ok(response)
    }

    pub(super) async fn set_plugin_config_setting(
        &self,
        plugin_id: &str,
        config_segments: &[String],
        value: JsonValue,
    ) -> Result<ConfigSettingsEditResponse> {
        let sources = self.config_json_sources()?;
        let mut record = plugin_record_for_config_edit(&sources, plugin_id);
        let config = normalize_plugin_record_for_config_edit(&mut record)?;
        set_nested_json_value(config, config_segments, value);
        let path = format!("plugins.list.{}", quoted_settings_segment(plugin_id));
        self.set_config_setting_direct(path.as_str(), record).await
    }

    pub(super) async fn delete_plugin_config_setting(
        &self,
        plugin_id: &str,
        config_segments: &[String],
    ) -> Result<ConfigSettingsEditResponse> {
        let sources = self.config_json_sources()?;
        let mut record = plugin_record_for_config_edit(&sources, plugin_id);
        let config = normalize_plugin_record_for_config_edit(&mut record)?;
        remove_nested_json_value(config, config_segments);
        let path = format!("plugins.list.{}", quoted_settings_segment(plugin_id));
        self.set_config_setting_direct(path.as_str(), record).await
    }
}

fn provider_summary_resource_from_catalog(
    provider: agena_provider::ProviderCatalogEntry,
    include_adapters: bool,
) -> ProviderSummaryResource {
    ProviderSummaryResource {
        provider_id: provider.provider_id.to_string(),
        defaults: ProviderDefaultsResource {
            adapter: provider.defaults.adapter,
            model: provider.defaults.model,
            thinking_mode: provider.defaults.thinking_mode,
            speed_mode: provider.defaults.speed_mode,
            verbosity: provider.defaults.verbosity,
            parallel_tool_calls: provider.defaults.parallel_tool_calls,
        },
        adapters: if include_adapters {
            provider.adapters
        } else {
            Vec::new()
        }
        .into_iter()
        .map(|adapter| ProviderAdapterSummaryResource {
            adapter_id: adapter.adapter_id,
            enabled: adapter.enabled,
            configured_model_count: adapter.configured_model_count,
        })
        .collect(),
        provider_native_tools: provider.provider_native_tools.map(|tools| {
            agena_api::resource::ProviderNativeToolsSummaryResource {
                active: tools.active,
                model_count: tools.model_count,
                bindings: tools
                    .bindings
                    .into_iter()
                    .map(
                        |binding| agena_api::resource::ProviderNativeToolBindingResource {
                            tool: binding.tool,
                            route: binding.route,
                        },
                    )
                    .collect(),
            }
        }),
    }
}
use crate::backend::Result;
use crate::backend::{
    ApiCommand, Application, Arc, Backend, CommandResult, ConfigJsonSources,
    ConfigSettingsEditResponse, CreateSessionParams, HashSet, JsonValue, ListSessionsParams,
    OnceLock, PaginatedResponse, PathBuf, ProviderAdapterSummaryResource, ProviderDefaultsResource,
    ProviderSummaryResource, Query, QueryResult, SessionResource, UpdateSessionParams, api_error,
    dispatch, env, fs, normalize_plugin_record_for_config_edit, parse_aws_profile_names,
    plugin_config_setting_target, plugin_record_for_config_edit, quoted_settings_segment,
    remove_nested_json_value, set_nested_json_value,
};
