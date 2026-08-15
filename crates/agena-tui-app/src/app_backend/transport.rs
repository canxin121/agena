//! Cloneable transport boundary between the TUI state machine and the owner
//! of session execution.
//!
//! `Embedded` exists for tests and explicit recovery/development launches.
//! `Remote` contains only the public API client plus client-local workspace
//! context; it never owns a Runtime, scheduler, provider client, session store,
//! or execution lease.

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
use agena_application::Application;
use agena_client::{AgenaClient, SubscriptionEvent};
use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;

use super::{LiveEvent, SessionRefresh};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendMode {
    Embedded,
    Remote,
}

#[derive(Clone)]
pub struct TuiBackend {
    inner: Arc<Backend>,
    workspace_root: Arc<PathBuf>,
}

enum Backend {
    Embedded(Application),
    Remote(RemoteBackend),
}

struct RemoteBackend {
    client: AgenaClient,
    workspace_id: i64,
    providers: Vec<ProviderSummaryResource>,
    models: HashMap<String, Vec<agena_domain::Model>>,
}

impl TuiBackend {
    pub fn embedded(application: Application) -> Self {
        let workspace_root = Arc::new(application.workspace_root().to_path_buf());
        Self {
            inner: Arc::new(Backend::Embedded(application)),
            workspace_root,
        }
    }

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
        Ok(Self {
            inner: Arc::new(Backend::Remote(RemoteBackend {
                client,
                workspace_id: workspace.id,
                providers,
                models,
            })),
            workspace_root: Arc::new(workspace_root),
        })
    }

    pub fn mode(&self) -> BackendMode {
        match self.inner.as_ref() {
            Backend::Embedded(_) => BackendMode::Embedded,
            Backend::Remote(_) => BackendMode::Remote,
        }
    }

    pub fn is_remote(&self) -> bool {
        self.mode() == BackendMode::Remote
    }

    pub fn workspace_root(&self) -> &Path {
        self.workspace_root.as_path()
    }

    pub(crate) fn embedded_application(&self) -> Result<&Application> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application),
            Backend::Remote(_) => Err(anyhow!(
                "this feature is unavailable in remote TUI mode because it has no public center API"
            )),
        }
    }

    fn embedded_feature(
        &self,
        feature: &str,
    ) -> std::result::Result<&Application, agena_application::ApplicationError> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application),
            Backend::Remote(_) => Err(agena_application::ApplicationError::internal(format!(
                "{feature} is unavailable in remote TUI mode because it has no public center API"
            ))),
        }
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
        self.embedded_feature("plugin Tool API")?
            .invoke_plugin_ui_tool(plugin_id, tool_name, input, session_id)
            .await
    }

    pub fn provider_config_draft(
        &self,
        provider_id: Option<&str>,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderConfigDraft,
        agena_application::ApplicationError,
    > {
        self.embedded_feature("Provider Studio")?
            .provider_config_draft(provider_id)
    }

    pub async fn list_draft_provider_adapter_models(
        &self,
        draft: &agena_application::provider_studio::ProviderConfigDraft,
        adapter_ids: &[String],
    ) -> std::result::Result<
        agena_api::resource::ProviderAdapterModelsResponse,
        agena_application::ApplicationError,
    > {
        self.embedded_feature("Provider Studio draft discovery")?
            .list_draft_provider_adapter_models(draft, adapter_ids)
            .await
    }

    pub async fn list_saved_provider_adapter_models(
        &self,
        provider_id: &str,
        adapter_ids: &[String],
    ) -> std::result::Result<
        agena_api::resource::ProviderAdapterModelsResponse,
        agena_application::ApplicationError,
    > {
        self.embedded_feature("Provider Studio adapter discovery")?
            .list_saved_provider_adapter_models(provider_id, adapter_ids)
            .await
    }

    pub fn provider_model_draft_value(
        &self,
        draft: &agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        provider_model: Option<&agena_api::resource::ProviderModelResource>,
    ) -> std::result::Result<serde_json::Value, agena_application::ApplicationError> {
        self.embedded_feature("Provider Studio model editor")?
            .provider_model_draft_value(draft, adapter_id, model_id, provider_model)
    }

    pub async fn save_provider_draft(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_model_lists: &[agena_api::resource::ProviderAdapterModelsResource],
        selected_adapter_ids: &[String],
        selected_model_keys: &std::collections::BTreeSet<String>,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        let application = self
            .embedded_application()
            .map_err(|_| remote_provider_studio_error())?;
        application
            .save_provider_draft(
                draft,
                adapter_model_lists,
                selected_adapter_ids,
                selected_model_keys,
            )
            .await
    }

    pub async fn save_provider_adapter_matches(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_models: agena_api::resource::ProviderAdapterModelsResource,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        let application = self
            .embedded_application()
            .map_err(|_| remote_provider_studio_error())?;
        application
            .save_provider_adapter_matches(draft, adapter_models)
            .await
    }

    pub async fn save_provider_model_value(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        model_value: serde_json::Value,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        let application = self
            .embedded_application()
            .map_err(|_| remote_provider_studio_error())?;
        application
            .save_provider_model_value(draft, adapter_id, model_id, model_value)
            .await
    }

    pub async fn delete_provider_model(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        let application = self
            .embedded_application()
            .map_err(|_| remote_provider_studio_error())?;
        application
            .delete_provider_model(draft, adapter_id, model_id)
            .await
    }

    pub async fn delete_provider(
        &self,
        provider_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        let application = self
            .embedded_application()
            .map_err(|_| remote_provider_studio_error())?;
        application.delete_provider(provider_id).await
    }

    pub async fn delete_provider_adapter(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        let application = self
            .embedded_application()
            .map_err(|_| remote_provider_studio_error())?;
        application.delete_provider_adapter(draft, adapter_id).await
    }

    pub async fn set_config_setting(
        &self,
        path: &str,
        value: serde_json::Value,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        self.embedded_feature("runtime configuration editing")?
            .set_config_setting(path, value)
            .await
    }

    pub async fn delete_config_setting(
        &self,
        path: &str,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        self.embedded_feature("runtime configuration editing")?
            .delete_config_setting(path)
            .await
    }

    pub async fn set_provider_default_selection(
        &self,
        provider_id: &str,
        selection: serde_json::Value,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        self.embedded_feature("provider default selection")?
            .set_provider_default_selection(provider_id, selection)
            .await
    }

    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: agena_domain::PermissionConfig,
    ) -> std::result::Result<SessionExecutionResource, agena_application::ApplicationError> {
        self.embedded_feature("session permission editing")?
            .set_session_permission(session_id, permission)
            .await
    }

    pub async fn session_overview(
        &self,
        workspace_id: Option<i64>,
        recent_limit: u64,
    ) -> Result<SessionOverviewResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application
                .session_overview(workspace_id, recent_limit)
                .await?),
            Backend::Remote(remote) => Ok(remote
                .client
                .session_overview(workspace_id.or(Some(remote.workspace_id)), recent_limit)
                .await?),
        }
    }

    pub async fn list_workspace_sessions(&self, roots_only: bool) -> Result<Vec<SessionResource>> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                Ok(application.list_workspace_sessions(roots_only).await?)
            }
            Backend::Remote(remote) => {
                let page = remote
                    .list_sessions(ListSessionsParams {
                        workspace_id: Some(remote.workspace_id),
                        roots: roots_only,
                        exclude_subagents: true,
                        limit: Some(200),
                        ..Default::default()
                    })
                    .await?;
                Ok(page.items)
            }
        }
    }

    pub async fn list_workspace_sessions_page(
        &self,
        roots_only: bool,
        exclude_subagents: bool,
        search: Option<&str>,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<PaginatedResponse<SessionResource>> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                super::operations::list_workspace_sessions_page_embedded(
                    application,
                    roots_only,
                    exclude_subagents,
                    search,
                    cursor,
                    limit,
                )
                .await
            }
            Backend::Remote(remote) => {
                remote
                    .list_sessions(ListSessionsParams {
                        workspace_id: Some(remote.workspace_id),
                        roots: roots_only,
                        exclude_subagents,
                        search: search.map(str::to_owned),
                        cursor,
                        limit: Some(limit),
                        ..Default::default()
                    })
                    .await
            }
        }
    }

    pub async fn list_child_sessions(&self, parent_id: i64) -> Result<Vec<SessionResource>> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                Ok(application.list_child_sessions(parent_id).await?)
            }
            Backend::Remote(remote) => Ok(remote
                .list_sessions(ListSessionsParams {
                    workspace_id: Some(remote.workspace_id),
                    parent_id: Some(parent_id),
                    limit: Some(200),
                    ..Default::default()
                })
                .await?
                .items),
        }
    }

    pub async fn list_session_subtree(&self, session_id: i64) -> Result<Vec<SessionResource>> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                Ok(application.list_session_subtree(session_id).await?)
            }
            Backend::Remote(remote) => {
                let mut cursor = remote.get_session(session_id).await?;
                while let Some(parent_id) = cursor.parent_id {
                    cursor = remote.get_session(parent_id).await?;
                }
                let result = remote
                    .client
                    .command(Command::ListSessionTree(
                        agena_api::commands::ListSessionTreeParams { root_id: cursor.id },
                    ))
                    .await?;
                let CommandResult::SessionTree(items) = result else {
                    bail!("processing center returned the wrong session-tree result");
                };
                Ok(items)
            }
        }
    }

    pub async fn rename_session(&self, session_id: i64, title: String) -> Result<SessionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                Ok(application.rename_session(session_id, title).await?)
            }
            Backend::Remote(remote) => {
                let result = remote
                    .client
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
        }
    }

    pub async fn create_session(
        &self,
        title: String,
        parent_id: Option<i64>,
    ) -> Result<SessionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                Ok(application.create_session(title, parent_id).await?)
            }
            Backend::Remote(remote) => Ok(remote
                .client
                .create_session(remote.workspace_id, title, parent_id)
                .await?),
        }
    }

    pub async fn get_session_state(&self, session_id: i64) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                Ok(application.session_execution_resource(session_id).await?)
            }
            Backend::Remote(remote) => Ok(remote.client.get_session_state(session_id).await?),
        }
    }

    pub async fn refresh_session(
        &self,
        session_id: i64,
        after_seq: Option<i64>,
        force: bool,
    ) -> Result<SessionRefresh> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                super::session_refresh::refresh_session_embedded(
                    application,
                    session_id,
                    after_seq,
                    force,
                )
                .await
            }
            Backend::Remote(remote) => {
                // REST/SSE is an invalidation protocol: the snapshot is the
                // correctness path. Reading on every refresh also converges
                // after SSE lag or reconnect without depending on replay.
                let execution = remote.client.get_session_state(session_id).await?;
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
        }
    }

    pub async fn submit_document(
        &self,
        session_id: i64,
        document: agena_domain::ComposerDocument,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application
                .submit_user_run(session_id, document, options)
                .await?),
            Backend::Remote(remote) => Ok(remote
                .client
                .submit_message(agena_api::commands::SubmitRunParams {
                    session_id,
                    options,
                    document,
                })
                .await?),
        }
    }

    pub async fn continue_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                Ok(application.continue_session(session_id, options).await?)
            }
            Backend::Remote(remote) => Ok(remote.client.continue_run(session_id, options).await?),
        }
    }

    pub async fn compact_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                Ok(application.compact_session(session_id, options).await?)
            }
            Backend::Remote(remote) => {
                Ok(remote.client.compact_session(session_id, options).await?)
            }
        }
    }

    pub async fn update_session_selection(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application
                .update_session_selection(session_id, options)
                .await?),
            Backend::Remote(remote) => Ok(remote
                .client
                .update_session_selection(session_id, options)
                .await?),
        }
    }

    pub async fn cancel_run(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::CancellationResult> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                Ok(application.cancel_run(session_id, execution_id).await?)
            }
            Backend::Remote(remote) => {
                Ok(remote.client.cancel_run(session_id, execution_id).await?)
            }
        }
    }

    pub async fn rewind_session_to_turn(
        &self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
    ) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application
                .rewind_session_to_turn(session_id, turn_id)
                .await?),
            Backend::Remote(remote) => {
                let result = remote
                    .client
                    .command(Command::RewindSession(RewindSessionParams {
                        session_id,
                        turn_id,
                        expected_version: None,
                    }))
                    .await?;
                execution_result(result, "session rewind")
            }
        }
    }

    pub async fn fork_session(
        &self,
        session_id: i64,
        title: Option<String>,
    ) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application
                .fork_session(session_id, None, title, None)
                .await?),
            Backend::Remote(remote) => {
                let result = remote
                    .client
                    .command(Command::ForkSession(ForkSessionParams {
                        session_id,
                        at_message_id: None,
                        title,
                    }))
                    .await?;
                execution_result(result, "session fork")
            }
        }
    }

    pub async fn reply_permission(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: PermissionReply,
    ) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application
                .reply_permission(session_id, options, reply, Some("tui".to_owned()))
                .await?),
            Backend::Remote(remote) => Ok(remote
                .client
                .reply_permission(agena_api::commands::ReplyPermissionParams {
                    session_id,
                    options,
                    reply,
                })
                .await?),
        }
    }

    pub async fn reply_user_input(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: UserInputReply,
    ) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application
                .reply_user_input(session_id, options, reply)
                .await?),
            Backend::Remote(remote) => Ok(remote
                .client
                .reply_user_input(agena_api::commands::ReplyUserInputParams {
                    session_id,
                    options,
                    reply,
                })
                .await?),
        }
    }

    pub async fn mark_interactive_request_presented(
        &self,
        session_id: i64,
        request_id: String,
    ) -> Result<SessionExecutionResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => Ok(application
                .mark_interactive_request_presented(session_id, request_id)
                .await?),
            Backend::Remote(remote) => Ok(remote
                .client
                .mark_interactive_request_presented(session_id, request_id.as_str())
                .await?),
        }
    }

    pub async fn list_providers(&self) -> Result<Vec<ProviderSummaryResource>> {
        Ok(self.provider_summaries())
    }

    pub(crate) fn provider_summaries(&self) -> Vec<ProviderSummaryResource> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                super::operations::list_configured_providers_embedded(application)
            }
            Backend::Remote(remote) => remote.providers.clone(),
        }
    }

    pub(crate) fn list_local_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<agena_domain::Model>> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => application
                .provider_catalog()
                .configured_local_models(&agena_domain::ProviderId::new(provider_id.trim()))
                .map_err(|error| anyhow!(error.to_string())),
            Backend::Remote(remote) => Ok(remote
                .models
                .get(provider_id.trim())
                .cloned()
                .unwrap_or_default()),
        }
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
        match self.inner.as_ref() {
            Backend::Embedded(application) => application.resolved_model_for_run_options(request),
            Backend::Remote(remote) => {
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
                remote
                    .providers
                    .first()
                    .and_then(|provider| {
                        remote
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
        }
    }

    pub fn resolved_model_default_modes(
        &self,
        request: &RunOptions,
    ) -> (Option<String>, Option<String>) {
        match self.inner.as_ref() {
            Backend::Embedded(application) => application.resolved_model_default_modes(request),
            Backend::Remote(_) => {
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
        }
    }

    /// Start a session-scoped invalidation stream. Establishing the remote SSE
    /// stream happens inside the spawned task so the synchronous TUI event
    /// handler never blocks. Any event causes snapshot convergence; lag and
    /// transport closure are also surfaced as forced refreshes.
    pub fn subscribe_session_events(&self, session_id: i64) -> mpsc::Receiver<LiveEvent> {
        match self.inner.as_ref() {
            Backend::Embedded(application) => {
                super::live_events::subscribe_session_events_embedded(application, session_id)
                    .unwrap_or_else(empty_live_receiver)
            }
            Backend::Remote(remote) => {
                let client = remote.client.clone();
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
        }
    }
}

impl RemoteBackend {
    async fn list_sessions(
        &self,
        params: ListSessionsParams,
    ) -> Result<PaginatedResponse<SessionResource>> {
        let result = self.client.query(Query::ListSessions(params)).await?;
        let QueryResult::Sessions(page) = result else {
            bail!("processing center returned the wrong session-list result");
        };
        Ok(page)
    }

    async fn get_session(&self, session_id: i64) -> Result<SessionResource> {
        let result = self
            .client
            .query(Query::GetSession(GetSessionParams { session_id }))
            .await?;
        let QueryResult::Session(session) = result else {
            bail!("processing center returned the wrong session result");
        };
        Ok(session)
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

fn empty_live_receiver() -> mpsc::Receiver<LiveEvent> {
    let (_tx, rx) = mpsc::channel(1);
    rx
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
            inner: Arc::new(Backend::Remote(RemoteBackend {
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
            })),
            workspace_root: Arc::new(PathBuf::from("/workspace")),
        };

        assert_eq!(backend.mode(), BackendMode::Remote);
        assert!(backend.embedded_application().is_err());
        let resolved = backend
            .resolved_model_for_run_options(&RunOptions::default())
            .expect("resolve cached remote default");
        assert_eq!(resolved.provider_id.as_ref(), "example");
        assert_eq!(resolved.model_id.as_ref(), "model-1");
    }
}
