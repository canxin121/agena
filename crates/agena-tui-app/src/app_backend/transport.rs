//! Cloneable transport boundary between the TUI state machine and the owner
//! of session execution.
//!
//! The TUI is a pure HTTP client. [`TuiBackend`] contains only the public API
//! client plus client-local workspace context; it never owns a Runtime,
//! scheduler, provider client, session store, or execution lease.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use agena_api::{
    commands::{
        Command, CommandResult, ForkSessionParams, RewindSessionParams, UpdateSessionParams,
    },
    pagination::PaginatedResponse,
    queries::{GetSessionParams, ListSessionsParams, Query, QueryResult},
    resource::{
        PermissionReply, ProviderSummaryResource, RunOptions, SessionExecutionResource,
        SessionOverviewResource, SessionResource, UserInputReply,
    },
};
use agena_client::{AgenaClient, SubscriptionEvent};
use anyhow::{Context, Result, bail};
use tokio::sync::mpsc;

use super::{LiveEvent, SessionRefresh};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendMode {
    Remote,
}

#[derive(Clone)]
pub struct TuiBackend {
    inner: Arc<RemoteBackend>,
    workspace_root: Arc<PathBuf>,
}

struct RemoteBackend {
    client: AgenaClient,
    workspace_id: i64,
    providers: Vec<ProviderSummaryResource>,
    models: HashMap<String, Vec<agena_domain::Model>>,
    /// Cached configuration-source read model assembled over HTTP. Loaded at
    /// connect and refreshed before settings-studio rebuilds; settings reads
    /// are synchronous in the TUI event loop.
    config_sources: tokio::sync::RwLock<Option<agena_application::dto::ConfigJsonSources>>,
    /// Cached plugin UI catalog (display contributions, theme palettes, slash
    /// commands) fetched from the center. Plugin reads are synchronous in the
    /// TUI event loop.
    plugin_catalog: tokio::sync::RwLock<Option<agena_plugin_host::PluginUiCatalog>>,
    /// Cached plugin statuses fetched from the center.
    plugin_statuses: tokio::sync::RwLock<Vec<agena_plugin_host::status::PluginStatus>>,
    /// Cached model-catalog page fetched from the center. The settings studio
    /// reads model counts synchronously inside the TUI event loop.
    model_catalog: tokio::sync::RwLock<Option<agena_application::dto::ModelCatalogListResponse>>,
}

impl TuiBackend {
    /// Connect to and validate a processing center, then resolve the local
    /// workspace path into the center's public workspace identity.
    pub async fn connect_remote(
        center_url: impl AsRef<str>,
        workspace_root: PathBuf,
    ) -> Result<Self> {
        Self::connect_remote_authenticated(center_url, workspace_root, None, None).await
    }

    pub async fn connect_remote_authenticated(
        center_url: impl AsRef<str>,
        workspace_root: PathBuf,
        center_token: Option<&str>,
        center_password: Option<&str>,
    ) -> Result<Self> {
        let client =
            AgenaClient::connect_center(center_url.as_ref(), center_token, center_password)
                .await
                .context("processing-center readiness/authentication handshake failed")?;
        let workspace = client
            .command(Command::ResolveWorkspace(
                agena_api::commands::ResolveWorkspaceParams {
                    path: workspace_root.to_string_lossy().into_owned(),
                    create_if_missing: true,
                },
            ))
            .await
            .context("failed to resolve the TUI workspace through the processing center")?;
        let CommandResult::Workspace(workspace) = workspace else {
            bail!("processing center returned the wrong result while resolving the workspace");
        };
        let providers = match client.query(Query::ListProviders).await? {
            QueryResult::Providers(providers) => providers,
            _ => bail!("processing center returned the wrong provider-list result"),
        };
        let mut models = HashMap::new();
        for provider in &providers {
            let response = client
                .query(Query::ListProviderModels(
                    agena_api::queries::ListProviderModelsParams {
                        provider_id: provider.provider_id.clone(),
                    },
                ))
                .await
                .with_context(|| {
                    format!(
                        "failed to load models for provider {} from the processing center",
                        provider.provider_id
                    )
                })?;
            let QueryResult::ProviderModels(response) = response else {
                bail!("processing center returned the wrong provider-model result");
            };
            let provider_models = response
                .models
                .into_iter()
                .map(provider_model_from_resource)
                .collect::<Result<Vec<_>>>()
                .with_context(|| {
                    format!(
                        "provider {} returned model metadata incompatible with this TUI",
                        provider.provider_id
                    )
                })?;
            models.insert(provider.provider_id.clone(), provider_models);
        }
        let backend = Self {
            inner: Arc::new(RemoteBackend {
                client,
                workspace_id: workspace.id,
                providers,
                models,
                config_sources: Default::default(),
                plugin_catalog: Default::default(),
                plugin_statuses: Default::default(),
                model_catalog: Default::default(),
            }),
            workspace_root: Arc::new(workspace_root),
        };
        // Config/plugin snapshots are presentation metadata; a failure here
        // must not prevent the client from connecting and driving sessions.
        let _ = backend.refresh_config_sources().await;
        let _ = backend.refresh_plugin_runtime_snapshot().await;
        Ok(backend)
    }

    pub fn mode(&self) -> BackendMode {
        BackendMode::Remote
    }

    pub fn is_remote(&self) -> bool {
        true
    }

    pub fn workspace_root(&self) -> &Path {
        self.workspace_root.as_path()
    }

    /// Access to the HTTP client for operations ported to REST.
    pub(crate) fn client(&self) -> &AgenaClient {
        &self.inner.client
    }

    /// The cached configuration-source read model, if it has been loaded from
    /// the center. Synchronous because settings presentation is built inside
    /// the TUI event loop.
    pub(crate) fn config_sources(&self) -> Option<agena_application::dto::ConfigJsonSources> {
        self.inner
            .config_sources
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// The resolved UI preferences projected from the center's effective
    /// configuration, for launching the terminal with the same
    /// theme/graphics/locale as an embedded runtime.
    pub fn tui_preferences(&self) -> agena_application::dto::TuiPreferencesResource {
        super::config::ui_configuration(self)
    }

    /// Refresh the cached configuration-source read model from the center:
    /// the global config file, the workspace config file, and the resolved
    /// effective document. `applied_layers` is not exposed over HTTP and is
    /// left empty (the settings studio then reports "built-in defaults").
    pub(crate) async fn refresh_config_sources(
        &self,
    ) -> Result<agena_application::dto::ConfigJsonSources> {
        use agena_application::dto::ConfigJsonSources;
        let client = &self.inner.client;
        let global = client.settings_layer_value("global", "").await?;
        let workspace = client.settings_layer_value("workspace", "").await?;
        let effective = client.resolved_config().await?;
        let config_path = global
            .get("config_path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_default();
        let config_found = global
            .get("config_found")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let project_config_path = workspace
            .get("config_path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_default();
        let project_config_found = workspace
            .get("config_found")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let sources = ConfigJsonSources {
            config_path,
            config_found,
            project_config_path,
            project_config_found,
            applied_layers: Vec::new(),
            file: global
                .get("value")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            project_file: workspace
                .get("value")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            effective,
        };
        *self.inner.config_sources.write().await = Some(sources.clone());
        Ok(sources)
    }

    /// The cached plugin UI catalog, if loaded from the center.
    pub(crate) fn plugin_catalog(&self) -> Option<agena_plugin_host::PluginUiCatalog> {
        self.inner
            .plugin_catalog
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// The cached plugin statuses, if loaded from the center.
    pub(crate) fn plugin_statuses(&self) -> Vec<agena_plugin_host::status::PluginStatus> {
        self.inner
            .plugin_statuses
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Refresh the cached plugin snapshot from the center: statuses and the
    /// combined TUI/studio UI catalog.
    pub(crate) async fn refresh_plugin_runtime_snapshot(&self) -> Result<()> {
        let client = &self.inner.client;
        let catalog_response = client.plugin_ui_catalog().await?;
        let catalog = catalog_response
            .get("catalog")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let catalog = serde_json::from_value::<agena_plugin_host::PluginUiCatalog>(catalog)
            .context("the center returned an undecodable plugin UI catalog")?;
        let statuses = serde_json::from_value::<Vec<agena_plugin_host::status::PluginStatus>>(
            client.plugin_statuses().await?,
        )
        .context("the center returned undecodable plugin statuses")?;
        *self.inner.plugin_catalog.write().await = Some(catalog);
        *self.inner.plugin_statuses.write().await = statuses;
        Ok(())
    }

    pub async fn invoke_plugin_ui_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> std::result::Result<
        agena_plugin_host::PluginUiToolInvokeResponse,
        agena_application::ApplicationError,
    > {
        let response = self
            .client()
            .invoke_plugin_ui_tool(plugin_id, tool_name, input, session_id)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to invoke plugin tool `{tool_name}` through the center: {error}"
                ))
            })?;
        serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "plugin tool `{tool_name}` returned a response this TUI cannot decode: {error}"
            ))
        })
    }

    /// Build the Provider Studio draft for `provider_id` (or a fresh draft for
    /// a not-yet-configured provider). No HTTP endpoint exposes the full draft
    /// projection, so this remains unavailable from a remote client.
    pub fn provider_config_draft(
        &self,
        _provider_id: Option<&str>,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderConfigDraft,
        agena_application::ApplicationError,
    > {
        Err(agena_application::ApplicationError::internal(
            "Provider Studio draft editing is unavailable in remote client mode because it has no public center API",
        ))
    }

    pub async fn list_draft_provider_adapter_models(
        &self,
        _draft: &agena_application::provider_studio::ProviderConfigDraft,
        _adapter_ids: &[String],
    ) -> std::result::Result<
        agena_api::resource::ProviderAdapterModelsResponse,
        agena_application::ApplicationError,
    > {
        Err(agena_application::ApplicationError::internal(
            "Provider Studio draft discovery is unavailable in remote client mode because it has no public center API",
        ))
    }

    pub async fn list_saved_provider_adapter_models(
        &self,
        provider_id: &str,
        adapter_ids: &[String],
    ) -> std::result::Result<
        agena_api::resource::ProviderAdapterModelsResponse,
        agena_application::ApplicationError,
    > {
        self.client()
            .list_saved_provider_adapter_models(
                provider_id,
                agena_api::resource::SavedProviderAdapterModelsRequest {
                    adapter_ids: adapter_ids.to_vec(),
                },
            )
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to list saved provider adapter models through the center: {error}"
                ))
            })
    }

    pub fn provider_model_draft_value(
        &self,
        _draft: &agena_application::provider_studio::ProviderConfigDraft,
        _adapter_id: &str,
        _model_id: &str,
        _provider_model: Option<&agena_api::resource::ProviderModelResource>,
    ) -> std::result::Result<serde_json::Value, agena_application::ApplicationError> {
        Err(agena_application::ApplicationError::internal(
            "Provider Studio model editing is unavailable in remote client mode because it has no public center API",
        ))
    }

    pub async fn save_provider_draft(
        &self,
        _draft: agena_application::provider_studio::ProviderConfigDraft,
        _adapter_model_lists: &[agena_api::resource::ProviderAdapterModelsResource],
        _selected_adapter_ids: &[String],
        _selected_model_keys: &std::collections::BTreeSet<String>,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        Err(remote_provider_studio_error())
    }

    pub async fn save_provider_adapter_matches(
        &self,
        _draft: agena_application::provider_studio::ProviderConfigDraft,
        _adapter_models: agena_api::resource::ProviderAdapterModelsResource,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        Err(remote_provider_studio_error())
    }

    pub async fn save_provider_model_value(
        &self,
        _draft: agena_application::provider_studio::ProviderConfigDraft,
        _adapter_id: &str,
        _model_id: &str,
        _model_value: serde_json::Value,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        Err(remote_provider_studio_error())
    }

    pub async fn delete_provider_model(
        &self,
        _draft: agena_application::provider_studio::ProviderConfigDraft,
        _adapter_id: &str,
        _model_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        Err(remote_provider_studio_error())
    }

    pub async fn delete_provider(
        &self,
        _provider_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        Err(remote_provider_studio_error())
    }

    pub async fn delete_provider_adapter(
        &self,
        _draft: agena_application::provider_studio::ProviderConfigDraft,
        _adapter_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        Err(remote_provider_studio_error())
    }

    /// Set a workspace-scoped config file setting through the center,
    /// reloading the runtime when the edit requires it.
    pub async fn set_config_setting(
        &self,
        path: &str,
        value: serde_json::Value,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        let response = self
            .client()
            .set_settings_layer_value("workspace", path.trim(), value, false, true)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to set workspace config setting `{path}` through the center: {error}"
                ))
            })?;
        serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the center returned an undecodable config edit response: {error}"
            ))
        })
    }

    /// Delete a workspace-scoped config file setting through the center,
    /// reloading the runtime when the edit requires it.
    pub async fn delete_config_setting(
        &self,
        path: &str,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        let response = self
            .client()
            .delete_settings_layer_value("workspace", path.trim(), false, true)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to delete workspace config setting `{path}` through the center: {error}"
                ))
            })?;
        serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the center returned an undecodable config edit response: {error}"
            ))
        })
    }

    /// Set the workspace's default provider selection (an alias of the
    /// `providers.default_selection` workspace config setting).
    pub async fn set_provider_default_selection(
        &self,
        provider_id: &str,
        selection: serde_json::Value,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        let _ = provider_id;
        self.set_config_setting("providers.default_selection", selection).await
    }

    /// Set a session's selected permission policy through the center.
    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: agena_domain::PermissionConfig,
    ) -> std::result::Result<SessionExecutionResource, agena_application::ApplicationError> {
        let resource: agena_api::resource::PermissionConfigResource = serde_json::from_value(
            serde_json::to_value(permission).map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to encode session permission: {error}"
                ))
            })?,
        )
        .map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "failed to encode session permission: {error}"
            ))
        })?;
        self.client()
            .set_session_permission(session_id, resource)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to update session permission through the center: {error}"
                ))
            })
    }

    pub async fn session_overview(
        &self,
        workspace_id: Option<i64>,
        recent_limit: u64,
    ) -> Result<SessionOverviewResource> {
        Ok(self
            .client()
            .session_overview(workspace_id.or(Some(self.inner.workspace_id)), recent_limit)
            .await?)
    }

    pub async fn list_workspace_sessions(&self, roots_only: bool) -> Result<Vec<SessionResource>> {
        let page = self
            .list_sessions(ListSessionsParams {
                workspace_id: Some(self.inner.workspace_id),
                roots: roots_only,
                exclude_subagents: true,
                limit: Some(200),
                ..Default::default()
            })
            .await?;
        Ok(page.items)
    }

    pub async fn list_workspace_sessions_page(
        &self,
        roots_only: bool,
        exclude_subagents: bool,
        search: Option<&str>,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<PaginatedResponse<SessionResource>> {
        self.list_sessions(ListSessionsParams {
            workspace_id: Some(self.inner.workspace_id),
            roots: roots_only,
            exclude_subagents,
            search: search.map(str::to_owned),
            cursor,
            limit: Some(limit),
            ..Default::default()
        })
        .await
    }

    pub async fn list_child_sessions(&self, parent_id: i64) -> Result<Vec<SessionResource>> {
        Ok(self
            .list_sessions(ListSessionsParams {
                workspace_id: Some(self.inner.workspace_id),
                parent_id: Some(parent_id),
                limit: Some(200),
                ..Default::default()
            })
            .await?
            .items)
    }

    pub async fn list_session_subtree(&self, session_id: i64) -> Result<Vec<SessionResource>> {
        let mut cursor = self.get_session(session_id).await?;
        while let Some(parent_id) = cursor.parent_id {
            cursor = self.get_session(parent_id).await?;
        }
        let result = self
            .client()
            .command(Command::ListSessionTree(
                agena_api::commands::ListSessionTreeParams { root_id: cursor.id },
            ))
            .await?;
        let CommandResult::SessionTree(items) = result else {
            bail!("processing center returned the wrong session-tree result");
        };
        Ok(items)
    }

    pub async fn rename_session(&self, session_id: i64, title: String) -> Result<SessionResource> {
        let result = self
            .client()
            .command(Command::UpdateSession(UpdateSessionParams {
                session_id,
                title,
                expected_version: None,
            }))
            .await?;
        let CommandResult::Session(session) = result else {
            bail!("processing center returned the wrong session-update result");
        };
        Ok(session)
    }

    pub async fn create_session(
        &self,
        title: String,
        parent_id: Option<i64>,
    ) -> Result<SessionResource> {
        Ok(self
            .client()
            .create_session(self.inner.workspace_id, title, parent_id)
            .await?)
    }

    pub async fn get_session_state(&self, session_id: i64) -> Result<SessionExecutionResource> {
        Ok(self.client().get_session_state(session_id).await?)
    }

    pub async fn refresh_session(
        &self,
        session_id: i64,
        after_seq: Option<i64>,
        force: bool,
    ) -> Result<SessionRefresh> {
        // REST/SSE is an invalidation protocol: the snapshot is the
        // correctness path. Reading on every refresh also converges
        // after SSE lag or reconnect without depending on replay.
        let _ = force;
        let execution = self.client().get_session_state(session_id).await?;
        let latest_event_seq = execution.latest_event_seq;
        let event_count = after_seq
            .zip(latest_event_seq)
            .map(|(after, current)| current.saturating_sub(after).clamp(0, 256) as usize)
            .unwrap_or(0);
        Ok(SessionRefresh {
            latest_event_seq,
            event_count,
            execution: Some(execution),
        })
    }

    pub async fn submit_document(
        &self,
        session_id: i64,
        document: agena_domain::ComposerDocument,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        Ok(self
            .client()
            .submit_message(agena_api::commands::SubmitRunParams {
                session_id,
                options,
                document,
            })
            .await?)
    }

    pub async fn continue_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        Ok(self.client().continue_run(session_id, options).await?)
    }

    pub async fn compact_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        Ok(self.client().compact_session(session_id, options).await?)
    }

    pub async fn update_session_selection(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        Ok(self
            .client()
            .update_session_selection(session_id, options)
            .await?)
    }

    pub async fn cancel_run(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::CancellationResult> {
        Ok(self.client().cancel_run(session_id, execution_id).await?)
    }

    pub async fn rewind_session_to_turn(
        &self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
    ) -> Result<SessionExecutionResource> {
        let result = self
            .client()
            .command(Command::RewindSession(RewindSessionParams {
                session_id,
                turn_id,
                expected_version: None,
            }))
            .await?;
        execution_result(result, "session rewind")
    }

    pub async fn fork_session(
        &self,
        session_id: i64,
        title: Option<String>,
    ) -> Result<SessionExecutionResource> {
        let result = self
            .client()
            .command(Command::ForkSession(ForkSessionParams {
                session_id,
                at_message_id: None,
                title,
            }))
            .await?;
        execution_result(result, "session fork")
    }

    pub async fn reply_permission(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: PermissionReply,
    ) -> Result<SessionExecutionResource> {
        Ok(self
            .client()
            .reply_permission(agena_api::commands::ReplyPermissionParams {
                session_id,
                options,
                reply,
            })
            .await?)
    }

    pub async fn reply_user_input(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: UserInputReply,
    ) -> Result<SessionExecutionResource> {
        Ok(self
            .client()
            .reply_user_input(agena_api::commands::ReplyUserInputParams {
                session_id,
                options,
                reply,
            })
            .await?)
    }

    pub async fn mark_interactive_request_presented(
        &self,
        session_id: i64,
        request_id: String,
    ) -> Result<SessionExecutionResource> {
        Ok(self
            .client()
            .mark_interactive_request_presented(session_id, request_id.as_str())
            .await?)
    }

    pub async fn list_providers(&self) -> Result<Vec<ProviderSummaryResource>> {
        Ok(self.provider_summaries())
    }

    pub(crate) fn provider_summaries(&self) -> Vec<ProviderSummaryResource> {
        self.inner.providers.clone()
    }

    pub(crate) fn list_local_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<agena_domain::Model>> {
        Ok(self
            .inner
            .models
            .get(provider_id.trim())
            .cloned()
            .unwrap_or_default())
    }

    /// The cached model-catalog page, if one has been loaded from the center.
    /// Synchronous because the settings studio reads model counts inside the
    /// TUI event loop. An empty response is returned when nothing is cached
    /// yet, so sync consumers can render without blocking on HTTP.
    pub(crate) fn model_catalog(
        &self,
    ) -> agena_application::dto::ModelCatalogListResponse {
        self.inner
            .model_catalog
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_else(agena_application::dto::ModelCatalogListResponse::empty)
    }

    /// Fetch a model-catalog page from the center and cache it for
    /// synchronous readers.
    pub(crate) async fn refresh_model_catalog_cache(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<()> {
        let value = self
            .inner
            .client
            .model_catalog(query, None, offset, limit)
            .await
            .context("failed to load the model catalog from the center")?;
        let response = serde_json::from_value(value)
            .context("the center returned an undecodable model catalog")?;
        *self.inner.model_catalog.write().await = Some(response);
        Ok(())
    }

    pub(crate) fn configured_model(
        &self,
        model: &agena_domain::ModelRef,
    ) -> Option<agena_domain::Model> {
        self.list_local_provider_models(model.provider_id.as_ref())
            .ok()?
            .into_iter()
            .find(|candidate| {
                candidate.id == model.model_id
                    && model
                        .adapter_id
                        .as_ref()
                        .is_none_or(|adapter_id| candidate.adapter_id.as_ref() == Some(adapter_id))
            })
    }

    pub fn resolved_model_for_run_options(
        &self,
        request: &RunOptions,
    ) -> std::result::Result<agena_domain::ModelRef, agena_application::ApplicationError> {
        if let Some(model) = request.model.as_ref() {
            return match model.adapter_id.as_deref() {
                Some(adapter_id) => agena_domain::ModelRef::try_new_with_adapter(
                    model.provider_id.as_str(),
                    adapter_id,
                    model.model_id.as_str(),
                ),
                None => agena_domain::ModelRef::try_new(
                    model.provider_id.as_str(),
                    model.model_id.as_str(),
                ),
            }
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "run option contains an invalid model reference: {error}"
                ))
            });
        }
        self.inner
            .providers
            .first()
            .and_then(|provider| {
                self.inner
                    .models
                    .get(provider.provider_id.as_str())
                    .and_then(|models| {
                        models
                            .iter()
                            .find(|model| model.id.as_ref() == provider.defaults.model)
                            .or_else(|| models.first())
                    })
            })
            .map(agena_domain::Model::reference)
            .ok_or_else(|| {
                agena_application::ApplicationError::internal(
                    "processing center exposes no configured model",
                )
            })
    }

    pub fn resolved_model_default_modes(
        &self,
        request: &RunOptions,
    ) -> (Option<String>, Option<String>) {
        let Ok(model_ref) = self.resolved_model_for_run_options(request) else {
            return (None, None);
        };
        let Some(model) = self.configured_model(&model_ref) else {
            return (None, None);
        };
        let thinking = model
            .thinking_modes
            .iter()
            .find(|mode| mode.is_default)
            .or_else(|| model.thinking_modes.first())
            .and_then(|mode| mode.selector().map(|selector| selector.into_owned()));
        let speed = model
            .speed_modes
            .iter()
            .find(|(_, mode)| mode.is_default)
            .or_else(|| model.speed_modes.iter().next())
            .map(|(name, _)| name.clone());
        (thinking, speed)
    }

    /// Start a session-scoped invalidation stream. Establishing the remote SSE
    /// stream happens inside the spawned task so the synchronous TUI event
    /// handler never blocks. Any event causes snapshot convergence; lag and
    /// transport closure are also surfaced as forced refreshes.
    pub fn subscribe_session_events(&self, session_id: i64) -> mpsc::Receiver<LiveEvent> {
        let client = self.client().clone();
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            loop {
                let connection = match client.connect_session(session_id).await {
                    Ok(connection) => connection,
                    Err(_) => {
                        if tx
                            .send(LiveEvent {
                                snapshot: None,
                                event: None,
                                force_refresh: true,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        tokio::select! {
                            _ = tx.closed() => return,
                            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                        }
                        continue;
                    }
                };
                if tx
                    .send(LiveEvent {
                        snapshot: Some(connection.snapshot),
                        event: None,
                        force_refresh: false,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                let mut subscription = connection.subscription;
                while let Some(item) = subscription.recv().await {
                    let force_refresh = match item {
                        Ok(SubscriptionEvent::SessionChanged(change)) => {
                            if change.session_id() != session_id {
                                continue;
                            }
                            false
                        }
                        Ok(SubscriptionEvent::RuntimeSignal(signal)) => {
                            if signal.session_id != Some(session_id) {
                                continue;
                            }
                            false
                        }
                        Ok(SubscriptionEvent::Lagged(_)) | Err(_) => true,
                    };
                    if tx
                        .send(LiveEvent {
                            snapshot: None,
                            event: None,
                            force_refresh,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if force_refresh {
                        break;
                    }
                }
                // A closed/lagged stream is not replayable. Invalidate,
                // then establish a fresh subscribe-before-snapshot pair.
                if tx
                    .send(LiveEvent {
                        snapshot: None,
                        event: None,
                        force_refresh: true,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::select! {
                    _ = tx.closed() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
                }
            }
        });
        rx
    }

    async fn list_sessions(
        &self,
        params: ListSessionsParams,
    ) -> Result<PaginatedResponse<SessionResource>> {
        let result = self.client().query(Query::ListSessions(params)).await?;
        let QueryResult::Sessions(page) = result else {
            bail!("processing center returned the wrong session-list result");
        };
        Ok(page)
    }

    async fn get_session(&self, session_id: i64) -> Result<SessionResource> {
        let result = self
            .client()
            .query(Query::GetSession(GetSessionParams { session_id }))
            .await?;
        let QueryResult::Session(session) = result else {
            bail!("processing center returned the wrong session result");
        };
        Ok(session)
    }

    /// Construct a backend pointed at a dead local endpoint for tests that
    /// only exercise synchronous routing/state effects (backend round-trips
    /// fail immediately and are surfaced as flashes).
    #[cfg(test)]
    pub(crate) fn remote_mock() -> Self {
        Self {
            inner: Arc::new(RemoteBackend {
                client: AgenaClient::new("http://127.0.0.1:9").expect("mock client"),
                workspace_id: 1,
                providers: Vec::new(),
                models: HashMap::new(),
                config_sources: Default::default(),
                plugin_catalog: Default::default(),
                plugin_statuses: Default::default(),
                model_catalog: Default::default(),
            }),
            workspace_root: Arc::new(PathBuf::from(std::env::temp_dir())),
        }
    }
}

fn execution_result(result: CommandResult, operation: &str) -> Result<SessionExecutionResource> {
    let CommandResult::Execution(execution) = result else {
        bail!("processing center returned the wrong result for {operation}");
    };
    Ok(execution)
}

fn provider_model_from_resource(
    model: agena_api::resource::ProviderModelResource,
) -> Result<agena_domain::Model> {
    let metadata = model.metadata;
    let value = serde_json::json!({
        "provider_id": model.provider_id,
        "adapter_id": model.adapter_id,
        "id": model.id,
        "catalog_model_id": model.catalog_model_id,
        "display_name": model.display_name,
        "native_compaction": model.native_compaction,
        // Capabilities are deliberately omitted here. The TUI model picker
        // needs route identity, labels, modes, and limits; the public API uses
        // a tri-state capability contract that must not be coerced back into
        // the runtime's internal boolean model.
        "metadata": {
            "lifecycle": metadata.lifecycle,
            "limits": {
                "context_window_tokens": metadata.context_window_tokens,
                "max_input_tokens": metadata.max_input_tokens,
                "max_output_tokens": metadata.max_output_tokens,
            },
            "description": metadata.description,
            "knowledge_cutoff": metadata.knowledge_cutoff,
            "release_date": metadata.release_date,
            "last_updated": metadata.last_updated,
            "open_weights": metadata.open_weights,
            "supports_parallel_tool_calls": metadata.supports_parallel_tool_calls,
            "supports_verbosity": metadata.supports_verbosity,
            "default_verbosity": metadata.default_verbosity,
            "default_temperature": metadata.default_temperature,
            "default_top_p": metadata.default_top_p,
            "default_top_k": metadata.default_top_k,
            "assistant_reasoning_interleaved": metadata.assistant_reasoning_interleaved,
            "assistant_reasoning_field": metadata.assistant_reasoning_field,
            "output_modalities": metadata.output_modalities,
            "pricing": metadata.pricing,
        },
        "thinking_modes": model.thinking_modes,
        "speed_modes": model.speed_modes,
    });
    serde_json::from_value(value).map_err(anyhow::Error::from)
}

fn remote_provider_studio_error() -> agena_application::provider_studio::ProviderStudioSaveError {
    let failure = agena_failure::Failure::new(
        agena_failure::FailureCode::new("tui.remote_feature_unavailable"),
        agena_failure::FailureCategory::DependencyUnavailable,
        agena_failure::FailureResponsibility::System,
        agena_failure::RetryDirective::AfterRefresh,
        agena_failure::RecoveryDirective::OpenSettings,
        agena_failure::FailureImpact::RequestRejected,
        agena_failure::UserPresentation::new(
            "tui.remote_feature_unavailable",
            "Provider Studio is unavailable in remote TUI mode until it has a public center API.",
        ),
    );
    agena_application::provider_studio::ProviderStudioSaveError::Other(failure.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_provider_model_maps_to_client_local_picker_metadata() {
        let resource = agena_api::resource::ProviderModelResource {
            provider_id: "example".to_owned(),
            adapter_id: Some("openai".to_owned()),
            id: "model-1".to_owned(),
            catalog_model_id: Some("catalog-1".to_owned()),
            display_name: Some("Model One".to_owned()),
            native_compaction: true,
            capabilities: Default::default(),
            metadata: agena_api::resource::ProviderModelMetadataResource {
                context_window_tokens: Some(128_000),
                supports_verbosity: Some(true),
                default_verbosity: Some("medium".to_owned()),
                ..Default::default()
            },
            thinking_modes: Vec::new(),
            speed_modes: Default::default(),
        };

        let model = provider_model_from_resource(resource).expect("map public model");
        assert_eq!(model.provider_id.as_ref(), "example");
        assert_eq!(model.adapter_id.as_ref().map(AsRef::as_ref), Some("openai"));
        assert_eq!(model.id.as_ref(), "model-1");
        assert_eq!(model.display_name.as_deref(), Some("Model One"));
        assert_eq!(model.metadata.limits.context_window_tokens, Some(128_000));
        assert_eq!(
            model
                .metadata
                .supported_verbosity_levels_for_model(&model.id),
            vec!["low", "medium", "high"]
        );
    }

    #[test]
    fn remote_backend_has_no_embedded_application_fallback() {
        let model = agena_domain::Model::new("example", "model-1");
        let backend = TuiBackend {
            inner: Arc::new(RemoteBackend {
                client: AgenaClient::new("http://127.0.0.1:9").expect("client"),
                workspace_id: 7,
                providers: vec![ProviderSummaryResource {
                    provider_id: "example".to_owned(),
                    defaults: agena_api::resource::ProviderDefaultsResource {
                        adapter: None,
                        model: "model-1".to_owned(),
                    },
                    adapters: Vec::new(),
                }],
                models: HashMap::from([("example".to_owned(), vec![model])]),
                config_sources: Default::default(),
                plugin_catalog: Default::default(),
                plugin_statuses: Default::default(),
                model_catalog: Default::default(),
            }),
            workspace_root: Arc::new(PathBuf::from("/workspace")),
        };

        assert_eq!(backend.mode(), BackendMode::Remote);
        let resolved = backend
            .resolved_model_for_run_options(&RunOptions::default())
            .expect("resolve cached remote default");
        assert_eq!(resolved.provider_id.as_ref(), "example");
        assert_eq!(resolved.model_id.as_ref(), "model-1");
    }
}
