use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use agena_notification::model::{Notification, NotificationScope};
use agena_provider::ProviderCatalog;

use crate::ApplicationError;
use crate::dto::{
    AuthBrowserStartResource, AuthCredentialIssuerResource, AuthCredentialType,
    AuthDeviceStartResource, AuthLoginKindResource, AuthLoginResultResource, AuthProviderResource,
    CatalogModelResource, ConfigJsonSources, ModelCatalogListResponse, ModelCatalogRefreshResponse,
    ModelCatalogResponse, ModelCatalogSourceKind, RuntimeBackgroundTaskResource,
    RuntimeDiagnosticsResource, RuntimeMetricsResource, RuntimeSnapshotSummaryResource,
    TuiPreferencesResource,
};
use crate::service::ApplicationService;

/// Shared in-process application handle.
///
/// Construction remains in the app/runtime composition layer. Presentation
/// code can obtain only the runtime capabilities it needs through this typed
/// handle instead of importing an HTTP/API-server state container.
#[derive(Clone)]
pub struct Application {
    workspace_root: PathBuf,
    provider_catalog: Arc<dyn ProviderCatalog>,
    model_catalog_runtime: Arc<dyn agena_runtime::ModelCatalogRuntimeService>,
    plugin_runtime: Arc<dyn agena_runtime::PluginRuntimeService>,
    runtime_configuration: Arc<dyn agena_runtime::RuntimeConfigurationService>,
    runtime_config_settings: Arc<dyn agena_runtime::RuntimeConfigSettingsService>,
    runtime_control: Arc<dyn agena_runtime::RuntimeControlService>,
    runtime_authentication: Arc<dyn agena_runtime::RuntimeAuthenticationService>,
    runtime_draft_authentication: Arc<dyn agena_runtime::RuntimeDraftAuthenticationService>,
    runtime_status: Arc<dyn agena_runtime::RuntimeStatusService>,
    runtime_activities: Option<Arc<dyn agena_runtime::RuntimeActivityService>>,
    event_queries: Option<Arc<dyn agena_runtime::RuntimeEventQueryService>>,
    event_stream: Option<Arc<dyn agena_runtime::RuntimeEventStreamService>>,
    service: ApplicationService,
    notifications: Arc<agena_runtime_notifications::store::InMemoryNotificationStore>,
    session_queries: Option<Arc<dyn agena_runtime::SessionQueryService>>,
    execution_control: Option<Arc<dyn agena_runtime::SessionExecutionControl>>,
    execution_commands: Option<Arc<dyn agena_runtime::SessionExecutionCommandService>>,
    tool_execution: Option<Arc<dyn agena_runtime::SessionToolExecutionService>>,
    plugin_commands: Option<Arc<dyn agena_runtime::SessionPluginCommandService>>,
}

/// Runtime session capabilities exposed to application command handling. The
/// legacy manager is adapted exactly once here while message/event projection
/// remains on its separate, explicitly concrete boundary.
#[derive(Clone)]
pub struct ApplicationSessionServices {
    pub execution_control: Arc<dyn agena_runtime::SessionExecutionControl>,
    pub queries: Arc<dyn agena_runtime::SessionQueryService>,
    pub commands: Arc<dyn agena_runtime::SessionExecutionCommandService>,
    pub tool_execution: Arc<dyn agena_runtime::SessionToolExecutionService>,
    pub plugin_commands: Arc<dyn agena_runtime::SessionPluginCommandService>,
}

/// Authentication flow selected by a transport or terminal command.
///
/// This is an Application input, deliberately distinct from Runtime's
/// provider-specific execution enum.  The conversion happens exactly once at
/// the Application boundary, so transports do not import Runtime auth types.
#[derive(Debug, Clone, Copy)]
pub enum AuthLoginKind {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
}

impl Application {
    /// Build an application handle from the complete Runtime-owned capability
    /// bundle. Normal consumers must use this path: Runtime has already
    /// selected the concrete storage adapters, while Application receives only
    /// storage contracts.
    pub fn from_composed_runtime_services(
        runtime: agena_runtime::RuntimeApplicationServices,
    ) -> Result<Self, ApplicationError> {
        let repositories = runtime.repositories.clone().ok_or_else(|| {
            ApplicationError::internal("runtime application repositories are unavailable")
        })?;
        let agena_runtime::RuntimeApplicationServices {
            workspace_root,
            repositories: _,
            provider_catalog,
            model_catalog,
            plugins,
            configuration,
            config_settings,
            control,
            authentication,
            draft_authentication,
            status,
            activities,
            tools: _,
            event_queries,
            event_stream,
            event_publisher,
            session_queries,
            execution_control,
            execution_commands,
            tool_execution,
            plugin_commands,
        } = runtime;
        let application = Self {
            provider_catalog,
            model_catalog_runtime: model_catalog,
            plugin_runtime: plugins,
            runtime_configuration: configuration,
            runtime_config_settings: config_settings,
            runtime_control: control,
            runtime_authentication: authentication,
            runtime_draft_authentication: draft_authentication,
            runtime_status: status,
            runtime_activities: activities,
            event_queries,
            event_stream,
            workspace_root: workspace_root.clone(),
            service: ApplicationService::new(
                workspace_root.display().to_string(),
                event_publisher,
                repositories.memory,
                repositories.workspace,
                repositories.permission_rules,
                repositories.session_stats,
                repositories.session_summary,
                repositories.session_mutation,
            ),
            notifications: Arc::new(
                agena_runtime_notifications::store::InMemoryNotificationStore::new(512),
            ),
            session_queries,
            execution_control,
            execution_commands,
            tool_execution,
            plugin_commands,
        };
        application.spawn_notification_aggregator();
        Ok(application)
    }

    /// Provider catalog port used by application-facing provider queries.
    pub fn provider_catalog(&self) -> &Arc<dyn ProviderCatalog> {
        &self.provider_catalog
    }

    /// Projects Runtime credential state into Application resources so
    /// transports never receive the Runtime authentication port for reads.
    pub fn auth_providers(&self) -> Result<Vec<AuthProviderResource>, ApplicationError> {
        self.runtime_authentication
            .auth_providers()
            .map(|providers| providers.into_iter().map(auth_provider_resource).collect())
            .map_err(application_error_from_runtime_authentication)
    }

    pub fn auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<AuthProviderResource, ApplicationError> {
        self.runtime_authentication
            .auth_provider(provider_id)
            .map(auth_provider_resource)
            .map_err(application_error_from_runtime_authentication)
    }

    pub async fn set_auth_api_key(
        &self,
        provider_id: &str,
        api_key: String,
    ) -> Result<AuthProviderResource, ApplicationError> {
        self.runtime_authentication
            .set_auth_api_key(provider_id, api_key)
            .map_err(application_error_from_runtime_authentication)?;
        self.reload_after_authentication_change().await?;
        self.auth_provider(provider_id)
    }

    pub async fn remove_auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<AuthProviderResource, ApplicationError> {
        self.runtime_authentication
            .remove_auth_provider(provider_id)
            .map_err(application_error_from_runtime_authentication)?;
        self.reload_after_authentication_change().await?;
        self.auth_provider(provider_id)
    }

    pub async fn refresh_auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<AuthProviderResource, ApplicationError> {
        self.runtime_authentication
            .refresh_auth_provider(provider_id)
            .await
            .map_err(application_error_from_runtime_authentication)?;
        self.reload_after_authentication_change().await?;
        self.auth_provider(provider_id)
    }

    pub async fn start_auth_browser(
        &self,
        provider_id: String,
        kind: AuthLoginKind,
        redirect_uri: String,
    ) -> Result<AuthBrowserStartResource, ApplicationError> {
        let start = self
            .runtime_authentication
            .start_auth_browser(
                provider_id.as_str(),
                runtime_auth_login_kind(kind),
                redirect_uri,
            )
            .await
            .map_err(application_error_from_runtime_authentication)?;
        Ok(AuthBrowserStartResource {
            provider_id,
            instance_url: start.instance_url,
            authorize_url: start.authorize_url,
            state: start.state,
            pkce_verifier: start.pkce_verifier,
        })
    }

    pub async fn finish_auth_browser(
        &self,
        provider_id: &str,
        kind: AuthLoginKind,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<AuthLoginResultResource, ApplicationError> {
        self.runtime_authentication
            .finish_auth_browser(
                provider_id,
                runtime_auth_login_kind(kind),
                code,
                pkce_verifier,
                redirect_uri,
            )
            .await
            .map_err(application_error_from_runtime_authentication)?;
        self.reload_after_authentication_change().await?;
        Ok(AuthLoginResultResource {
            completed: true,
            provider: Some(self.auth_provider(provider_id)?),
        })
    }

    /// Wait for the Runtime-owned local callback and complete the browser
    /// login as one Application lifecycle.  Terminal code retains prompting
    /// and timeout selection, but cannot receive Runtime callback values or
    /// invoke the authentication port directly.
    pub async fn complete_auth_browser_callback(
        &self,
        provider_id: &str,
        kind: AuthLoginKind,
        port: u16,
        expected_state: &str,
        timeout: std::time::Duration,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<AuthLoginResultResource, ApplicationError> {
        let callback = self
            .runtime_authentication
            .wait_auth_browser_callback(port, expected_state, timeout)
            .map_err(application_error_from_runtime_authentication)?;
        self.finish_auth_browser(
            provider_id,
            kind,
            callback.code,
            pkce_verifier,
            redirect_uri,
        )
        .await
    }

    pub async fn start_auth_device(
        &self,
        provider_id: String,
        kind: AuthLoginKind,
        enterprise_domain: Option<String>,
    ) -> Result<AuthDeviceStartResource, ApplicationError> {
        let start = self
            .runtime_authentication
            .start_auth_device(
                provider_id.as_str(),
                runtime_auth_login_kind(kind),
                enterprise_domain.clone(),
            )
            .await
            .map_err(application_error_from_runtime_authentication)?;
        Ok(AuthDeviceStartResource {
            provider_id,
            enterprise_domain,
            verification_url: start.verification_url,
            user_code: start.user_code,
            device_code: start.device_code,
            interval_seconds: start.interval_seconds,
        })
    }

    pub async fn poll_auth_device(
        &self,
        provider_id: &str,
        kind: AuthLoginKind,
        device_code: String,
        user_code: Option<String>,
        enterprise_domain: Option<String>,
    ) -> Result<AuthLoginResultResource, ApplicationError> {
        let completed = self
            .runtime_authentication
            .poll_auth_device(
                provider_id,
                runtime_auth_login_kind(kind),
                device_code,
                user_code,
                enterprise_domain,
            )
            .await
            .map_err(application_error_from_runtime_authentication)?;
        if completed {
            self.reload_after_authentication_change().await?;
        }
        Ok(AuthLoginResultResource {
            completed,
            provider: completed
                .then(|| self.auth_provider(provider_id))
                .transpose()?,
        })
    }

    /// Returns the Application-owned catalog projection used by terminal
    /// presentation. Runtime still owns refresh, curation, and storage; App
    /// must not duplicate the Runtime response-to-display projection.
    pub fn list_model_catalog(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> ModelCatalogListResponse {
        self.list_model_catalog_with_origin(query, None, offset, limit)
    }

    /// Lists the Application-owned catalog projection with an optional origin
    /// filter. Transport adapters receive only this resource, never the
    /// Runtime catalog snapshot service.
    pub fn list_model_catalog_with_origin(
        &self,
        query: &str,
        origin: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> ModelCatalogListResponse {
        let catalog = self.model_catalog_runtime.model_catalog_response();
        let summary = model_catalog_summary(self.model_catalog_runtime.as_ref(), &catalog);
        let models = model_catalog_resources(&catalog);
        let search = query.trim().to_lowercase();
        let origin = origin
            .map(str::trim)
            .filter(|origin| !origin.is_empty() && *origin != "all");
        let available_origins = models
            .iter()
            .filter_map(|model| {
                let origin = model.origin.as_deref()?.trim();
                (!origin.is_empty()).then(|| origin.to_owned())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let filtered = models
            .into_iter()
            .filter(|model| {
                (origin
                    .map(|origin| model.origin.as_deref().map(str::trim) == Some(origin))
                    .unwrap_or(true))
                    && (search.is_empty()
                        || model_catalog_search_text(model).contains(search.as_str()))
            })
            .collect::<Vec<_>>();
        let total = filtered.len();
        let limit = crate::pagination::normalize_limit(Some(limit as u64)) as usize;
        let items = filtered.into_iter().skip(offset).take(limit).collect();

        ModelCatalogListResponse {
            summary,
            total,
            offset,
            limit,
            available_origins,
            items,
        }
    }

    /// Resolves raw and canonical catalog IDs into Application resources.
    pub fn lookup_model_catalog_models(&self, model_ids: &[String]) -> Vec<CatalogModelResource> {
        let requested = model_ids
            .iter()
            .flat_map(|model_id| {
                let raw = model_id.trim().to_owned();
                if raw.is_empty() {
                    return Vec::new();
                }
                let canonical = agena_provider::normalized_catalog_model_id(raw.as_str());
                if canonical.is_empty() || canonical == raw {
                    vec![raw]
                } else {
                    vec![raw, canonical]
                }
            })
            .collect::<BTreeSet<_>>();
        let catalog = self.model_catalog_runtime.model_catalog_response();
        model_catalog_resources(&catalog)
            .into_iter()
            .filter(|model| requested.contains(model.model_id.as_str()))
            .collect()
    }

    /// Starts the user-requested catalog refresh without leaking Runtime task
    /// origin or task-construction values to App presentation code.
    pub fn request_model_catalog_refresh(&self) -> Result<(), ApplicationError> {
        self.refresh_model_catalog().map(|_| ())
    }

    /// Starts a user-requested catalog refresh and returns the Application
    /// task/summary resource required by HTTP and terminal presentation.
    pub fn refresh_model_catalog(&self) -> Result<ModelCatalogRefreshResponse, ApplicationError> {
        let task = self
            .model_catalog_runtime
            .start_model_catalog_refresh(agena_runtime::RuntimeBackgroundTaskOrigin::User)
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let catalog = self.model_catalog_runtime.model_catalog_response();
        Ok(ModelCatalogRefreshResponse {
            started: task.started,
            task: RuntimeBackgroundTaskResource::from(task.task),
            summary: model_catalog_summary(self.model_catalog_runtime.as_ref(), &catalog),
        })
    }

    /// Refreshes the configured provider client versions as one product use
    /// case: fetch current versions, persist the Runtime settings patch, and
    /// reload only when the settings service requires it.
    pub async fn refresh_provider_client_versions(
        &self,
    ) -> Result<agena_provider::ProviderClientVersions, ApplicationError> {
        let versions = self
            .runtime_control
            .fetch_provider_client_versions()
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let response = self
            .runtime_config_settings
            .patch_file_settings(agena_runtime::ConfigSettingsPatchInput {
                target: agena_runtime::ConfigSettingsPathInput {
                    path: Some("runtime.providers.client_versions".to_owned()),
                },
                changes: serde_json::json!({
                    "codex": versions.codex,
                    "claude": versions.claude,
                    "gemini": versions.gemini,
                }),
                options: agena_runtime::ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })
            .map_err(|error| ApplicationError::internal(error.to_string()))?;

        if response.reload_required {
            self.runtime_control
                .reload()
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?;
        }
        Ok(versions)
    }

    /// Stable read-only plugin-host operations for application transports.
    pub fn plugin_runtime(&self) -> &Arc<dyn agena_runtime::PluginRuntimeService> {
        &self.plugin_runtime
    }

    /// Configuration-file editing capabilities adapted by the composed runtime.
    pub fn runtime_config_settings(&self) -> &Arc<dyn agena_runtime::RuntimeConfigSettingsService> {
        &self.runtime_config_settings
    }

    pub fn runtime_control(&self) -> &Arc<dyn agena_runtime::RuntimeControlService> {
        &self.runtime_control
    }

    /// Unified background-activity service, when the composed runtime exposes
    /// one.
    pub fn runtime_activities(
        &self,
    ) -> Result<Arc<dyn agena_runtime::RuntimeActivityService>, ApplicationError> {
        self.runtime_activities
            .clone()
            .ok_or_else(|| ApplicationError::internal("background activity service is unavailable"))
    }

    /// Unified notification store shared by API transports and the TUI.
    pub fn notifications(
        &self,
    ) -> &Arc<agena_runtime_notifications::store::InMemoryNotificationStore> {
        &self.notifications
    }

    /// Wire the runtime event stream into the unified notification store.
    ///
    /// Spawns a background projection task that converts user-visible runtime
    /// events (notice parts and background-activity changes) into unified
    /// notifications. Outside a Tokio runtime the call is a no-op, so CLI
    /// helpers that compose an `Application` for one-shot queries stay safe.
    pub fn spawn_notification_aggregator(&self) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let Some(stream_service) = self.event_stream.clone() else {
            return;
        };
        let store = Arc::clone(&self.notifications);
        handle.spawn(async move {
            let filter = agena_domain::EventFilter {
                scope: agena_domain::EventScope::Global,
                kinds: Some(
                    [
                        agena_domain::EventKindTag::from("message_part_checkpointed"),
                        agena_domain::EventKindTag::from("background_activity_changed"),
                    ]
                    .into_iter()
                    .collect(),
                ),
                since_seq_global: None,
            };
            let mut subscription = stream_service.subscribe_events(filter);
            while let Some(item) = subscription.recv().await {
                let event = match item {
                    agena_runtime::RuntimeLiveEventSubscriptionItem::Event(event) => event,
                    agena_runtime::RuntimeLiveEventSubscriptionItem::Lagged(_) => continue,
                };
                if let Some(notification) = notification_from_runtime_event(&event) {
                    store.ingest(notification);
                }
            }
        });
    }

    pub fn runtime_draft_authentication(
        &self,
    ) -> &Arc<dyn agena_runtime::RuntimeDraftAuthenticationService> {
        &self.runtime_draft_authentication
    }

    /// Internal dispatch access for the existing full Runtime wire-status
    /// query. Application consumers use dedicated projections instead.
    pub(crate) fn runtime_status(&self) -> &Arc<dyn agena_runtime::RuntimeStatusService> {
        &self.runtime_status
    }

    /// Projects the small runtime-health summary needed by terminal
    /// diagnostics without exposing Runtime status values to App code.
    pub async fn runtime_snapshot_summary(&self) -> RuntimeSnapshotSummaryResource {
        let status = self.runtime_status.runtime_status().await;
        RuntimeSnapshotSummaryResource {
            generation: status.generation,
            loaded_at: status.loaded_at,
            provider_count: status.provider_ids.len(),
            plugin_count: status.plugin_count,
        }
    }

    /// Projects the complete Studio diagnostic surface without exposing a
    /// RuntimeApplicationServices bundle or Runtime status record to Studio.
    pub async fn runtime_diagnostics(&self) -> RuntimeDiagnosticsResource {
        let status = self.runtime_status.runtime_status().await;
        RuntimeDiagnosticsResource {
            generation: status.generation,
            loaded_at: status.loaded_at,
            workspace_root: status.workspace_root,
            config_path: status.config_path,
            config_found: status.config_found,
            provider_ids: status.provider_ids,
            session_runtime_available: status.session_runtime_available,
        }
    }

    /// Projects process-wide Runtime counters for transport presentation.
    pub fn runtime_metrics(&self) -> RuntimeMetricsResource {
        self.runtime_control.runtime_metrics().into()
    }

    /// Projects Runtime-owned persisted terminal preferences for startup and
    /// palette reload without exposing Runtime configuration values to the App.
    pub fn tui_preferences(&self) -> Result<TuiPreferencesResource, ApplicationError> {
        self.runtime_configuration
            .runtime_configuration()
            .map(|configuration| configuration.ui.into())
            .map_err(|error| ApplicationError::internal(error.to_string()))
    }

    /// Returns the complete configuration-source read model used by terminal
    /// settings, provider, permission, and plugin presentation.
    pub fn config_json_sources(&self) -> Result<ConfigJsonSources, ApplicationError> {
        let configuration = self
            .runtime_configuration
            .runtime_configuration()
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        let file = self
            .runtime_config_settings
            .read_file_settings(agena_runtime::ConfigSettingsGetInput::default())
            .map_err(|error| ApplicationError::internal(error.to_string()))?
            .value;
        let project_file = self
            .runtime_config_settings
            .read_project_file_settings(agena_runtime::ConfigSettingsGetInput::default())
            .map_err(|error| ApplicationError::internal(error.to_string()))?
            .value;
        let mut effective = configuration.effective_config;
        augment_effective_config_json(&mut effective, configuration.default_provider.as_deref());

        Ok(ConfigJsonSources {
            config_path: configuration.config_path,
            config_found: configuration.config_found,
            project_config_path: configuration.project_config_path,
            project_config_found: configuration.project_config_found,
            applied_layers: configuration.applied_layers,
            file,
            project_file,
            effective,
        })
    }

    /// Resolves the active configuration path for an editor/process effect.
    pub fn config_path(&self) -> Result<PathBuf, ApplicationError> {
        self.runtime_configuration
            .runtime_configuration()
            .map(|configuration| configuration.config_path)
            .map_err(|error| ApplicationError::internal(error.to_string()))
    }

    pub fn service(&self) -> &ApplicationService {
        &self.service
    }

    /// Returns the application-owned snapshot projection for the composed
    /// session runtime. Callers must not inspect Runtime's snapshot registry
    /// directly: registry availability and the stable presentation shape are
    /// application-service concerns.
    pub fn snapshot_status(&self) -> crate::dto::SnapshotStatusResource {
        let capabilities = agena_runtime::RuntimeControlService::snapshot_backend_capabilities(
            self.runtime_control.as_ref(),
            &self.workspace_root,
        );
        self.service
            .snapshot_status(self.execution_control.as_deref(), capabilities)
    }

    /// Returns the application-owned source-control projection. The concrete
    /// Runtime execution-control port is supplied only at this composition
    /// boundary so CLI, TUI, and transport consumers share the same status
    /// policy and snapshot accounting.
    pub async fn git_status(&self) -> Result<crate::dto::GitStatusResource, ApplicationError> {
        self.service
            .git_status(self.execution_control.as_deref())
            .await
    }

    /// Initializes source control through the application use case while
    /// retaining snapshot accounting at the application boundary.
    pub async fn git_init(&self) -> Result<crate::dto::GitStatusResource, ApplicationError> {
        self.service
            .git_init(self.execution_control.as_deref())
            .await
    }

    /// Returns the raw version-control patch used by the application-facing
    /// review flow.
    pub async fn vcs_diff_raw(&self) -> Result<String, ApplicationError> {
        self.service.vcs_diff_raw().await
    }

    /// Stages workspace changes through the application-owned Git use case.
    pub async fn git_stage(
        &self,
        request: crate::dto::GitStageRequest,
    ) -> Result<crate::dto::GitStatusResource, ApplicationError> {
        self.service
            .git_stage(self.execution_control.as_deref(), request)
            .await
    }

    /// Creates a commit through the application-owned Git use case.
    pub async fn git_commit(
        &self,
        request: crate::dto::GitCommitRequest,
    ) -> Result<crate::dto::GitCommitResource, ApplicationError> {
        self.service
            .git_commit(self.execution_control.as_deref(), request)
            .await
    }

    /// Creates a pull request through the application-owned Git use case.
    pub async fn git_create_pull_request(
        &self,
        request: crate::dto::GitPullRequestCreateRequest,
    ) -> Result<crate::dto::GitPullRequestResource, ApplicationError> {
        self.service
            .git_create_pull_request(self.execution_control.as_deref(), request)
            .await
    }

    pub fn workspace_root(&self) -> &std::path::Path {
        self.workspace_root.as_path()
    }

    /// Read-only session projections are exposed independently from the
    /// concrete manager so header-only message queries do not materialize it.
    pub fn session_query_service(
        &self,
    ) -> Result<Arc<dyn agena_runtime::SessionQueryService>, ApplicationError> {
        self.session_queries.clone().ok_or_else(|| {
            ApplicationError::service_unavailable("session query service not initialised")
        })
    }

    pub fn session_execution_services(
        &self,
    ) -> Result<ApplicationSessionServices, ApplicationError> {
        Ok(ApplicationSessionServices {
            execution_control: self.execution_control.clone().ok_or_else(|| {
                ApplicationError::service_unavailable("session runtime not initialised")
            })?,
            queries: self.session_query_service()?,
            commands: self.execution_commands.clone().ok_or_else(|| {
                ApplicationError::service_unavailable("session runtime not initialised")
            })?,
            tool_execution: self.tool_execution.clone().ok_or_else(|| {
                ApplicationError::service_unavailable("session runtime not initialised")
            })?,
            plugin_commands: self.plugin_commands.clone().ok_or_else(|| {
                ApplicationError::service_unavailable("session runtime not initialised")
            })?,
        })
    }

    pub fn event_stream_service(
        &self,
    ) -> Result<Arc<dyn agena_runtime::RuntimeEventStreamService>, ApplicationError> {
        self.event_stream.clone().ok_or_else(|| {
            ApplicationError::service_unavailable("event stream service not initialised")
        })
    }

    pub fn event_query_service(
        &self,
    ) -> Result<Arc<dyn agena_runtime::RuntimeEventQueryService>, ApplicationError> {
        self.event_queries.clone().ok_or_else(|| {
            ApplicationError::service_unavailable("event query service not initialised")
        })
    }
}

impl Application {
    async fn reload_after_authentication_change(&self) -> Result<(), ApplicationError> {
        self.runtime_control
            .reload()
            .await
            .map(|_| ())
            .map_err(|error| ApplicationError::internal(error.to_string()))
    }
}

fn model_catalog_summary(
    runtime: &dyn agena_runtime::ModelCatalogRuntimeService,
    catalog: &agena_provider::ModelCatalogResponse,
) -> ModelCatalogResponse {
    ModelCatalogResponse {
        refreshing: runtime.model_catalog_refresh_active(),
        last_refresh_at: catalog.last_refresh_at,
        last_successful_source: catalog.last_successful_source,
        last_failure: catalog.last_failure.as_ref().map(Into::into),
        model_count: catalog.models.len(),
    }
}

fn application_error_from_runtime_authentication(
    error: agena_runtime::RuntimeAuthenticationError,
) -> ApplicationError {
    match error.kind {
        agena_runtime::RuntimeAuthenticationErrorKind::BadRequest => {
            ApplicationError::bad_request_with_diagnostic(
                "The authentication request is invalid.",
                error.message,
            )
        }
        agena_runtime::RuntimeAuthenticationErrorKind::NotFound => {
            ApplicationError::not_found_with_diagnostic(
                "The authentication method was not found.",
                error.message,
            )
        }
        agena_runtime::RuntimeAuthenticationErrorKind::Internal => {
            ApplicationError::internal(error.message)
        }
    }
}

fn runtime_auth_login_kind(kind: AuthLoginKind) -> agena_runtime::RuntimeAuthLoginKind {
    match kind {
        AuthLoginKind::OpenaiChatgpt => agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt,
        AuthLoginKind::GithubCopilot => agena_runtime::RuntimeAuthLoginKind::GithubCopilot,
        AuthLoginKind::Gitlab => agena_runtime::RuntimeAuthLoginKind::Gitlab,
    }
}

impl From<AuthLoginKindResource> for AuthLoginKind {
    fn from(kind: AuthLoginKindResource) -> Self {
        match kind {
            AuthLoginKindResource::OpenaiChatgpt => Self::OpenaiChatgpt,
            AuthLoginKindResource::GithubCopilot => Self::GithubCopilot,
            AuthLoginKindResource::Gitlab => Self::Gitlab,
        }
    }
}

fn auth_provider_resource(provider: agena_runtime::RuntimeAuthProvider) -> AuthProviderResource {
    let expires_at = provider.expires_at;
    AuthProviderResource {
        provider_id: provider.provider_id,
        configured: true,
        credential_present: provider.credential_present,
        credential_type: provider.credential_type.map(|value| match value {
            agena_runtime::RuntimeAuthCredentialType::Api => AuthCredentialType::Api,
            agena_runtime::RuntimeAuthCredentialType::Oauth => AuthCredentialType::Oauth,
            agena_runtime::RuntimeAuthCredentialType::WellKnown => AuthCredentialType::WellKnown,
        }),
        credential_issuer: provider.credential_issuer.map(|value| match value {
            agena_runtime::RuntimeAuthCredentialIssuer::OpenaiChatgpt => {
                AuthCredentialIssuerResource::OpenaiChatgpt
            }
            agena_runtime::RuntimeAuthCredentialIssuer::GithubCopilot => {
                AuthCredentialIssuerResource::GithubCopilot
            }
            agena_runtime::RuntimeAuthCredentialIssuer::Gitlab => {
                AuthCredentialIssuerResource::Gitlab
            }
            agena_runtime::RuntimeAuthCredentialIssuer::GoogleAdc => {
                AuthCredentialIssuerResource::GoogleAdc
            }
            agena_runtime::RuntimeAuthCredentialIssuer::SapAiCore => {
                AuthCredentialIssuerResource::SapAiCore
            }
        }),
        key_preview: provider.key_preview,
        expires_at,
        expired: expires_at.is_some_and(|value| value <= chrono::Utc::now()),
        account_id: provider.account_id,
        enterprise_url: provider.enterprise_url,
        username: provider.username,
        display_name: provider.display_name,
        email: provider.email,
        avatar_url: provider.avatar_url,
        api_key_write_supported: provider.api_key_write_supported,
        refresh_supported: provider.browser_login_kind.is_some(),
        browser_login_kind: provider.browser_login_kind.map(|value| match value {
            agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt => {
                AuthLoginKindResource::OpenaiChatgpt
            }
            agena_runtime::RuntimeAuthLoginKind::GithubCopilot => {
                AuthLoginKindResource::GithubCopilot
            }
            agena_runtime::RuntimeAuthLoginKind::Gitlab => AuthLoginKindResource::Gitlab,
        }),
        browser_login_instance_url: provider.browser_login_instance_url,
        device_login_kind: provider.device_login_kind.map(|value| match value {
            agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt => {
                AuthLoginKindResource::OpenaiChatgpt
            }
            agena_runtime::RuntimeAuthLoginKind::GithubCopilot => {
                AuthLoginKindResource::GithubCopilot
            }
            agena_runtime::RuntimeAuthLoginKind::Gitlab => AuthLoginKindResource::Gitlab,
        }),
    }
}

fn augment_effective_config_json(
    effective: &mut serde_json::Value,
    default_provider: Option<&str>,
) {
    if let Some(provider) = default_provider {
        set_effective_config_alias(
            effective,
            &["providers", "default"],
            serde_json::Value::String(provider.to_owned()),
        );
    }
}

fn set_effective_config_alias(
    root: &mut serde_json::Value,
    segments: &[&str],
    value: serde_json::Value,
) {
    if segments.is_empty() {
        *root = value;
        return;
    }
    if !root.is_object() {
        *root = serde_json::Value::Object(serde_json::Map::new());
    }
    let mut cursor = root;
    for segment in &segments[..segments.len().saturating_sub(1)] {
        let object = cursor.as_object_mut().expect("effective config object");
        cursor = object
            .entry((*segment).to_owned())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if !cursor.is_object() {
            *cursor = serde_json::Value::Object(serde_json::Map::new());
        }
    }
    let object = cursor.as_object_mut().expect("effective config object");
    object.insert(segments[segments.len() - 1].to_owned(), value);
}

fn model_catalog_resources(
    catalog: &agena_provider::ModelCatalogResponse,
) -> Vec<CatalogModelResource> {
    catalog
        .models
        .iter()
        .cloned()
        .map(|model| CatalogModelResource::from_record(model, catalog.last_successful_source))
        .collect()
}

fn model_catalog_search_text(model: &CatalogModelResource) -> String {
    let thinking_mode_text = model
        .thinking_modes
        .iter()
        .flat_map(|(name, mode)| {
            [
                name.clone(),
                mode.display_name.clone().unwrap_or_default(),
                mode.description.clone().unwrap_or_default(),
                mode.thinking
                    .as_ref()
                    .and_then(|value| serde_json::to_string(value).ok())
                    .unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n");
    let speed_mode_text = model
        .speed_modes
        .iter()
        .flat_map(|(name, mode)| {
            [
                name.clone(),
                mode.display_name.clone().unwrap_or_default(),
                mode.description.clone().unwrap_or_default(),
                serde_json::to_string(&mode.request_override).unwrap_or_default(),
                serde_json::to_string(&mode.adapter_overrides).unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n");

    [
        model.model_id.clone(),
        model.display_name.clone().unwrap_or_default(),
        model.origin.clone().unwrap_or_default(),
        model.description.clone().unwrap_or_default(),
        model
            .context_window_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        model
            .max_input_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        model
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        match model.source {
            ModelCatalogSourceKind::Generated => "generated".to_owned(),
            ModelCatalogSourceKind::Cache => "cache".to_owned(),
        },
        model.source_label.clone().unwrap_or_default(),
        model
            .lifecycle
            .map(|value| match value {
                agena_domain::ModelLifecycle::Active => "active",
                agena_domain::ModelLifecycle::Preview => "preview",
                agena_domain::ModelLifecycle::Beta => "beta",
                agena_domain::ModelLifecycle::Alpha => "alpha",
                agena_domain::ModelLifecycle::Experimental => "experimental",
                agena_domain::ModelLifecycle::Deprecated => "deprecated",
            })
            .unwrap_or_default()
            .to_owned(),
        thinking_mode_text,
        speed_mode_text,
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase()
}

/// Project one live runtime event into a unified notification, or `None` when
/// the event carries no user-visible notice (transcript content, streaming
/// deltas, and non-notice activities are intentionally not notification-worthy).
fn notification_from_runtime_event(event: &agena_runtime::RuntimeEvent) -> Option<Notification> {
    match event.kind.as_str() {
        "message_part_checkpointed" => {
            let payload: agena_runtime::event::MessagePartCheckpointedEvent =
                serde_json::from_value(event.payload.clone()).ok()?;
            let content = payload.part.content?;
            let agena_runtime::message::PartContent::Activity(
                agena_runtime::message::RuntimeActivity::Notice(part),
            ) = content
            else {
                return None;
            };
            Some(agena_runtime_notifications::from_notice_part(
                &part,
                NotificationScope::Session(payload.session_id),
                event.meta.created_at.timestamp_millis(),
            ))
        }
        "background_activity_changed" => {
            let payload: agena_domain::BackgroundActivityChangedEvent =
                serde_json::from_value(event.payload.clone()).ok()?;
            Some(agena_runtime_notifications::from_background_activity(
                &payload.activity,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod notification_aggregator_tests {
    use super::*;
    use agena_domain::{
        BackgroundActivity, BackgroundActivityEventReason, BackgroundActivityKind,
        BackgroundActivityStatus, EventMeta, ExecutionStatus, Role,
    };
    use agena_notification::model::{NotificationKind, NotificationSeverity, NotificationSurface};
    use agena_runtime::event::MessagePartCheckpointedEvent;
    use agena_runtime::message::{MessagePart, NoticePart, PartContent, RuntimeActivity};
    use chrono::Utc;
    use uuid::Uuid;

    fn meta(session_id: Option<i64>) -> EventMeta {
        EventMeta {
            id: Uuid::new_v4(),
            seq_global: 1,
            seq_session: session_id.map(|_| 1),
            session_id,
            workspace_id: None,
            created_at: Utc::now(),
            causation_id: None,
            correlation_id: None,
            envelope_schema: 1,
        }
    }

    #[test]
    fn notice_part_event_projects_to_session_banner() {
        let part = MessagePart::from_content(
            1,
            1,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::Activity(RuntimeActivity::Notice(NoticePart {
                kind: "max_turns_exhausted".to_owned(),
                summary: "Turn budget exhausted".to_owned(),
                detail: Some("Reduce scope".to_owned()),
            })),
        );
        let payload = MessagePartCheckpointedEvent {
            session_id: 7,
            execution_id: None,
            run_id: None,
            turn_id: None,
            reply_id: None,
            message_id: 1,
            message_role: Role::Assistant,
            message_state: ExecutionStatus::Completed,
            message_created_at: Utc::now(),
            message_metadata: Default::default(),
            part,
            ts_ms: 1,
        };
        let event = agena_runtime::RuntimeEvent {
            meta: meta(Some(7)),
            kind: "message_part_checkpointed".to_owned(),
            payload: serde_json::to_value(&payload).expect("serialize payload"),
            invalidates_ancestor_projection: false,
        };
        let notification = notification_from_runtime_event(&event).expect("projected");
        assert_eq!(
            notification.kind,
            NotificationKind::Notice {
                code: "max_turns_exhausted".into()
            }
        );
        assert_eq!(notification.scope, NotificationScope::Session(7));
        assert_eq!(notification.surface, NotificationSurface::Banner);
        assert_eq!(notification.summary, "Turn budget exhausted");
    }

    #[test]
    fn background_activity_event_projects_to_activities_panel() {
        let activity = BackgroundActivity {
            id: "task_1".to_owned(),
            kind: BackgroundActivityKind::Task,
            status: BackgroundActivityStatus::Failed,
            title: "Run tests".to_owned(),
            description: "cargo test".to_owned(),
            command: None,
            workdir: None,
            session_id: Some(7),
            parent_session_id: None,
            created_at_ms: 1000,
            started_at_ms: 1000,
            finished_at_ms: None,
            exit_code: None,
            message: None,
            failure: None,
            last_seq: 0,
            has_more: false,
            dropped_lines: 0,
            cancellable: true,
            dismissible: true,
        };
        let event = agena_runtime::RuntimeEvent {
            meta: meta(Some(7)),
            kind: "background_activity_changed".to_owned(),
            payload: serde_json::to_value(agena_domain::BackgroundActivityChangedEvent {
                activity_id: "task_1".to_owned(),
                reason: BackgroundActivityEventReason::Finished,
                activity,
                ts_ms: 1000,
            })
            .expect("serialize payload"),
            invalidates_ancestor_projection: false,
        };
        let notification = notification_from_runtime_event(&event).expect("projected");
        assert_eq!(notification.severity, NotificationSeverity::Error);
        assert_eq!(
            notification.kind,
            NotificationKind::BackgroundActivity {
                activity_id: "task_1".into()
            }
        );
        assert_eq!(notification.surface, NotificationSurface::ActivitiesPanel);
    }

    #[test]
    fn unrelated_events_project_to_none() {
        let event = agena_runtime::RuntimeEvent {
            meta: meta(None),
            kind: "execution_started".to_owned(),
            payload: serde_json::json!({}),
            invalidates_ancestor_projection: false,
        };
        assert!(notification_from_runtime_event(&event).is_none());
    }
}
