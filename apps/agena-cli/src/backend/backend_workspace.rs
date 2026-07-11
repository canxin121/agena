use anyhow::{Context, anyhow};

impl Backend {
    pub fn new(
        runtime: AgenaRuntime,
        db: Arc<DatabaseConnection>,
        workspace_root: PathBuf,
    ) -> Self {
        Self {
            app_state: AppState::new(runtime.clone(), db),
            runtime,
            workspace_root,
            file_index: Arc::new(OnceLock::new()),
        }
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
            &self.app_state,
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
            .context("failed to resolve workspace for agena-tui")?;

        match dispatch::dispatch_command(
            &self.app_state,
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
            &self.app_state,
            ApiCommand::UpdateSession(UpdateSessionParams {
                session_id,
                title,
                parent_id: existing.parent_id,
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
        let snapshot = self.runtime.current_snapshot();
        let registry = snapshot.provider_registry();
        let mut providers = registry
            .provider_ids()
            .into_iter()
            .filter_map(|provider_id| {
                registry.get(provider_id.as_str()).map(|provider| {
                    let configured = snapshot
                        .config_resolution()
                        .config
                        .providers
                        .get(provider_id.as_str());
                    ProviderSummaryResource {
                        defaults: ProviderDefaultsResource {
                            adapter: provider.default_adapter().map(ToString::to_string),
                            model: provider.default_model().to_string(),
                            thinking_mode: configured
                                .and_then(|provider| provider.defaults.thinking_mode.clone()),
                            speed_mode: configured
                                .and_then(|provider| provider.defaults.speed_mode.clone()),
                            verbosity: configured
                                .and_then(|provider| provider.defaults.verbosity.clone()),
                            parallel_tool_calls: configured
                                .and_then(|provider| provider.defaults.parallel_tool_calls),
                        },
                        adapters: Vec::new(),
                        native_tools: configured.map(provider_native_tools_summary_resource),
                        provider_id,
                    }
                })
            })
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        providers
    }

    pub fn list_agent_names(&self) -> Vec<String> {
        let snapshot = self.runtime.current_snapshot();
        let mut names = snapshot.agents().names();
        names.sort();
        names
    }

    pub fn list_agent_descriptors(&self) -> Vec<AgentDescriptor> {
        self.runtime.current_snapshot().agents().list_descriptors()
    }

    pub fn get_agent_profile(&self, name: &str) -> Option<agena::agents::AgentProfile> {
        self.runtime.current_snapshot().agents().get(name.trim())
    }

    pub fn config_has_agent(&self, name: &str) -> bool {
        self.runtime
            .current_snapshot()
            .config_resolution()
            .config
            .agents
            .contains_key(name.trim())
    }

    pub fn config_agent_names(&self) -> HashSet<String> {
        self.runtime
            .current_snapshot()
            .config_resolution()
            .config
            .agents
            .keys()
            .cloned()
            .collect()
    }

    pub fn default_agent_name(&self) -> Option<String> {
        let snapshot = self.runtime.current_snapshot();
        let configured = snapshot
            .config_resolution()
            .config
            .default_agent
            .clone()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let mut agents = snapshot.agents().list_descriptors();
        agents.sort_by(|left, right| left.name.cmp(&right.name));

        if !configured.is_empty() && agents.iter().any(|agent| agent.name == configured) {
            return Some(configured);
        }

        agents.into_iter().map(|agent| agent.name).next()
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
        let snapshot = self.runtime.current_snapshot();
        let mut providers = snapshot
            .config_resolution()
            .config
            .providers
            .iter()
            .map(|(provider_id, provider)| ProviderSummaryResource {
                provider_id: provider_id.clone(),
                defaults: ProviderDefaultsResource {
                    adapter: provider.defaults.adapter.clone(),
                    model: provider.defaults.model.clone().unwrap_or_default(),
                    thinking_mode: provider.defaults.thinking_mode.clone(),
                    speed_mode: provider.defaults.speed_mode.clone(),
                    verbosity: provider.defaults.verbosity.clone(),
                    parallel_tool_calls: provider.defaults.parallel_tool_calls,
                },
                adapters: provider
                    .adapters
                    .iter()
                    .map(|(adapter_id, adapter)| ProviderAdapterSummaryResource {
                        adapter_id: adapter_id.clone(),
                        enabled: adapter.enabled,
                        configured_model_count: provider
                            .models
                            .keys()
                            .filter(|model_id| {
                                model_id
                                    .split_once('/')
                                    .map(|(route_adapter_id, _)| route_adapter_id == adapter_id)
                                    .unwrap_or(false)
                            })
                            .count(),
                    })
                    .collect(),
                native_tools: Some(provider_native_tools_summary_resource(provider)),
            })
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        providers
    }

    pub fn config_path(&self) -> PathBuf {
        self.runtime.config_resolution().meta.config_path.clone()
    }

    pub fn config_json_sources(&self) -> Result<ConfigJsonSources> {
        let snapshot = self.runtime.current_snapshot();
        let resolution = snapshot.config_resolution();
        let config_path = resolution.meta.config_path.clone();
        let file = read_file_setting(config_path.clone(), ConfigSettingsGetInput::default())
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to read config file settings")?
            .value;
        let project_config_path = resolution.meta.project_config_path.clone();
        let project_file = read_file_setting(
            project_config_path.clone(),
            ConfigSettingsGetInput::default(),
        )
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to read workspace config file settings")?
        .value;
        let mut effective = serde_json::to_value(&resolution.config)
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to serialize effective config")?;
        augment_effective_config_json(&mut effective, &resolution.config);
        Ok(ConfigJsonSources {
            config_path,
            config_found: resolution.meta.config_found,
            project_config_path,
            project_config_found: resolution.meta.project_config_found,
            applied_layers: resolution
                .meta
                .applied_layers
                .iter()
                .map(|layer| layer.description.clone())
                .collect(),
            file,
            project_file,
            effective,
        })
    }

    pub async fn set_config_setting(
        &self,
        path: &str,
        value: JsonValue,
    ) -> Result<ConfigSettingsEditResponse> {
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
    ) -> Result<ConfigSettingsEditResponse> {
        let config_path = self.runtime.config_resolution().meta.config_path.clone();
        let response = set_file_setting(
            config_path,
            ConfigSettingsSetInput {
                path: path.trim().to_owned(),
                value,
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            },
        )
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to set config setting")?;

        if response.reload_required {
            self.runtime
                .reload()
                .await
                .context("failed to reload runtime after config change")?;
        }
        Ok(response)
    }

    pub async fn reload_runtime(&self) -> Result<()> {
        self.runtime
            .reload()
            .await
            .context("failed to reload runtime after agent source change")?;
        Ok(())
    }

    pub async fn delete_config_setting(&self, path: &str) -> Result<ConfigSettingsEditResponse> {
        if let Some((plugin_id, config_segments)) = plugin_config_setting_target(path)? {
            return self
                .delete_plugin_config_setting(plugin_id.as_str(), config_segments.as_slice())
                .await;
        }
        let config_path = self.runtime.config_resolution().meta.config_path.clone();
        let response = delete_file_setting(
            config_path,
            ConfigSettingsDeleteInput {
                path: path.trim().to_owned(),
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            },
        )
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to delete config setting")?;

        if response.reload_required {
            self.runtime
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
    ) -> Result<ConfigSettingsEditResponse> {
        let config_path = self
            .runtime
            .config_resolution()
            .meta
            .project_config_path
            .clone();
        let response = set_file_setting(
            config_path,
            ConfigSettingsSetInput {
                path: path.trim().to_owned(),
                value,
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            },
        )
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to set workspace config setting")?;

        if response.reload_required {
            self.runtime
                .reload()
                .await
                .context("failed to reload runtime after workspace config change")?;
        }
        Ok(response)
    }

    pub async fn delete_workspace_config_setting(
        &self,
        path: &str,
    ) -> Result<ConfigSettingsEditResponse> {
        let config_path = self
            .runtime
            .config_resolution()
            .meta
            .project_config_path
            .clone();
        let response = delete_file_setting(
            config_path,
            ConfigSettingsDeleteInput {
                path: path.trim().to_owned(),
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            },
        )
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to delete workspace config setting")?;

        if response.reload_required {
            self.runtime
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
use crate::backend::Result;
use crate::backend::{
    AgenaRuntime, AgentDescriptor, ApiCommand, AppState, Arc, Backend, CommandResult,
    ConfigJsonSources, ConfigSettingsDeleteInput, ConfigSettingsEditOptions,
    ConfigSettingsEditResponse, ConfigSettingsGetInput, ConfigSettingsSetInput,
    CreateSessionParams, DatabaseConnection, HashSet, JsonValue, ListSessionsParams, OnceLock,
    PaginatedResponse, PathBuf, ProviderAdapterSummaryResource, ProviderDefaultsResource,
    ProviderSummaryResource, Query, QueryResult, SessionResource, UpdateSessionParams, api_error,
    augment_effective_config_json, delete_file_setting, dispatch, env, fs,
    normalize_plugin_record_for_config_edit, parse_aws_profile_names, plugin_config_setting_target,
    plugin_record_for_config_edit, provider_native_tools_summary_resource, quoted_settings_segment,
    read_file_setting, remove_nested_json_value, set_file_setting, set_nested_json_value,
};
