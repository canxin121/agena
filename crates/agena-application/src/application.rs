use std::sync::atomic::{AtomicBool, Ordering};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use agena_notification::model::NotificationScope;
use agena_provider::ProviderCatalog;

use crate::ApplicationError;
use crate::dto::{
    AuthBrowserStartResource, AuthCredentialIssuerResource, AuthCredentialType,
    AuthDeviceStartResource, AuthLoginKindResource, AuthLoginResultResource, AuthProviderResource,
    CatalogModelMatchResource, CatalogModelResource, ConfigJsonSources, ModelCatalogListResponse,
    ModelCatalogRefreshResponse, ModelCatalogResponse, ModelCatalogSourceKind,
    RuntimeDiagnosticsResource, RuntimeMetricsResource, RuntimeSnapshotSummaryResource,
};
use crate::service::ApplicationService;

static APPLICATION_FILE_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

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
    runtime_tools: Arc<dyn agena_runtime::RuntimeToolExecutionService>,
    runtime_activities: Option<Arc<dyn agena_runtime::RuntimeActivityService>>,
    live_signals: Option<Arc<dyn agena_runtime::RuntimeLiveSignalService>>,
    service: ApplicationService,
    notifications: Arc<agena_runtime_notifications::store::InMemoryNotificationStore>,
    notification_aggregator: Arc<NotificationAggregator>,
    session_queries: Option<Arc<dyn agena_runtime::SessionQueryService>>,
    execution_control: Option<Arc<dyn agena_runtime::SessionExecutionControl>>,
    execution_commands: Option<Arc<dyn agena_runtime::SessionExecutionCommandService>>,
    tool_execution: Option<Arc<dyn agena_runtime::SessionToolExecutionService>>,
    plugin_commands: Option<Arc<dyn agena_runtime::SessionPluginCommandService>>,
    pub(crate) part_documents: Arc<crate::application_tools::PartDocumentCache>,
}

#[derive(Default)]
struct NotificationAggregator {
    started: AtomicBool,
    tasks: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Drop for NotificationAggregator {
    fn drop(&mut self) {
        let tasks = self
            .tasks
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for task in tasks.drain(..) {
            task.abort();
        }
    }
}

/// Runtime session capabilities exposed to application command handling.
/// Message/event projection remains on its separate, explicitly concrete
/// boundary.
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
    /// Execute a synchronous application projection or persisted edit outside
    /// Tokio's workers. The owned application keeps its selected service
    /// handles alive, and the bounded pool also covers cancelled callers.
    pub async fn run_blocking<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Self) -> T + Send + 'static,
    ) -> Result<T, ApplicationError> {
        let application = self.clone();
        APPLICATION_FILE_WORKERS
            .run(move || operation(&application))
            .await
            .map_err(|error| {
                ApplicationError::internal(format!("application worker failed: {error}"))
            })
    }

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
            tools,
            live_signals,
            session_kernel,
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
            runtime_tools: tools,
            runtime_activities: activities,
            live_signals,
            workspace_root: workspace_root.clone(),
            service: ApplicationService::new(
                workspace_root.display().to_string(),
                repositories.memory,
                repositories.workspace,
                repositories.permission_rules,
                session_kernel.ok_or_else(|| {
                    ApplicationError::internal("runtime session kernel is unavailable")
                })?,
            ),
            notifications: Arc::new(
                agena_runtime_notifications::store::InMemoryNotificationStore::new(512),
            ),
            notification_aggregator: Arc::new(NotificationAggregator::default()),
            session_queries,
            execution_control,
            execution_commands,
            tool_execution,
            plugin_commands,
            part_documents: Arc::new(Default::default()),
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
        let stored_provider_id = provider_id.to_owned();
        self.run_blocking(move |app| {
            app.runtime_authentication
                .set_auth_api_key(&stored_provider_id, api_key)
        })
        .await?
        .map_err(application_error_from_runtime_authentication)?;
        self.reload_after_authentication_change().await?;
        self.auth_provider(provider_id)
    }

    pub async fn remove_auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<AuthProviderResource, ApplicationError> {
        let stored_provider_id = provider_id.to_owned();
        self.run_blocking(move |app| {
            app.runtime_authentication
                .remove_auth_provider(&stored_provider_id)
        })
        .await?
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
        // The runtime owns an async listener; waiting for a browser redirect
        // consumes neither a blocking-pool slot nor an additional runtime.
        let callback = self
            .runtime_authentication
            .wait_auth_browser_callback(port, expected_state, timeout)
            .await
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
    ///
    /// Lists with an optional origin filter. Transport adapters receive only
    /// this resource, never the Runtime catalog snapshot service.
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

    /// Suggest a catalog entry for each raw provider ID without changing the
    /// provider model or recording a permanent association.
    pub fn match_model_catalog_models(
        &self,
        model_ids: &[String],
    ) -> Vec<CatalogModelMatchResource> {
        let candidates = model_ids
            .iter()
            .map(|model_id| crate::provider_studio::catalog::catalog_lookup_candidates(model_id))
            .collect::<Vec<_>>();
        let lookup_ids = candidates.iter().flatten().cloned().collect::<Vec<_>>();
        let entries = self.lookup_model_catalog_models(&lookup_ids);
        model_ids
            .iter()
            .zip(candidates)
            .map(|(model_id, candidates)| CatalogModelMatchResource {
                model_id: model_id.clone(),
                catalog: crate::provider_studio::catalog::preferred_catalog_model_for_lookup_ids(
                    &entries,
                    &candidates,
                )
                .cloned(),
            })
            .collect()
    }

    /// Starts a user-requested catalog refresh and returns the Application
    /// task/summary resource required by HTTP and terminal presentation.
    pub fn refresh_model_catalog(&self) -> Result<ModelCatalogRefreshResponse, ApplicationError> {
        let task = self
            .model_catalog_runtime
            .start_model_catalog_refresh(agena_runtime::RuntimeBackgroundTaskOrigin::User)
            .map_err(ApplicationError::from)?;
        let catalog = self.model_catalog_runtime.model_catalog_response();
        Ok(ModelCatalogRefreshResponse {
            started: task.started,
            task: crate::dto::runtime_background_task_resource(task.task),
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
            .map_err(|error| ApplicationError::internal_error(&error))?;
        let auto_update = self
            .runtime_configuration
            .runtime_configuration()
            .map_err(|error| ApplicationError::internal_error(&error))?
            .effective_config
            .pointer("/runtime/providers/client_versions/auto_update")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
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
                    "auto_update": auto_update,
                }),
                options: agena_runtime::ConfigSettingsEditOptions {
                    expected_revision: None,
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })
            .map_err(|error| ApplicationError::internal_error(&error))?;

        if response.reload_required {
            self.runtime_control
                .reload()
                .await
                .map_err(ApplicationError::from)?;
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

    /// Wire live surfaces into the unified notification store.
    ///
    /// Projects committed kernel notices and runtime activity changes into
    /// unified notifications without loading transcript history. Outside a Tokio runtime the call is a no-op, so CLI
    /// helpers that compose an `Application` for one-shot queries stay safe.
    pub fn spawn_notification_aggregator(&self) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self
            .notification_aggregator
            .started
            .swap(true, Ordering::AcqRel)
        {
            return;
        }
        if let Ok(kernel) = self.session_kernel() {
            let reader = kernel.reader.clone();
            if let Ok(mut watch) = reader.watch_all(None) {
                let store = Arc::clone(&self.notifications);
                let task = handle.spawn(async move {
                    loop {
                        let change = match watch.recv().await {
                            Ok(change) => change,
                            Err(agena_storage::kernel::WatchError::Closed) => break,
                            Err(agena_storage::kernel::WatchError::Reset { .. }) => {
                                match reader.watch_all(None) {
                                    Ok(next) => watch = next,
                                    Err(_) => break,
                                }
                                continue;
                            }
                        };
                        for id in &change.affected_entries {
                            let Ok(entry) = reader.entry(change.session, *id).await else {
                                continue;
                            };
                            let agena_storage::kernel::EntryPayload::Notice { text } =
                                &entry.payload
                            else {
                                continue;
                            };
                            let notice =
                                serde_json::from_str::<agena_runtime_contracts::NoticePart>(text)
                                    .unwrap_or_else(|_| agena_runtime_contracts::NoticePart {
                                        kind: "session_notice".into(),
                                        summary: text.clone(),
                                        detail: None,
                                        title: None,
                                    });
                            let mut notification = agena_runtime_notifications::from_notice_part(
                                &notice,
                                NotificationScope::Session(change.session.get()),
                                entry.created_at_ms,
                            );
                            notification.id =
                                format!("kernel-notice-{}-{}", change.session.get(), id.get());
                            notification.dedup_key = Some(notification.id.clone());
                            if let Err(error) = store.ingest_with(move || notification).await {
                                tracing::error!(%error, "session notice notification failed");
                            }
                        }
                    }
                });
                self.notification_aggregator
                    .tasks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(task);
            }
        }
        let store = Arc::clone(&self.notifications);
        let Some(live_signals) = self.live_signals.clone() else {
            return;
        };
        let task = handle.spawn(async move {
            let mut subscription = live_signals.subscribe();
            while let Some(item) = subscription.recv().await {
                tokio::task::consume_budget().await;
                let signal = match item {
                    agena_runtime::RuntimeLiveSignalItem::Signal(signal) => signal,
                    agena_runtime::RuntimeLiveSignalItem::Lagged(_) => continue,
                };
                if let agena_runtime::RuntimeLiveSignal::Activity(activity) = signal
                    && let Err(error) = store
                        .ingest_with(move || {
                            agena_runtime_notifications::from_background_activity(
                                &activity.activity,
                            )
                        })
                        .await
                {
                    tracing::error!(%error, "background activity notification failed");
                }
            }
        });
        self.notification_aggregator
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(task);
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

    pub(crate) fn runtime_configuration(
        &self,
    ) -> &Arc<dyn agena_runtime::RuntimeConfigurationService> {
        &self.runtime_configuration
    }

    pub(crate) fn runtime_tools(&self) -> &Arc<dyn agena_runtime::RuntimeToolExecutionService> {
        &self.runtime_tools
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

    /// Projects the complete server diagnostic surface without exposing a
    /// RuntimeApplicationServices bundle or Runtime status record to transports.
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

    /// Returns the complete configuration-source read model used by terminal
    /// settings, provider, permission, and plugin presentation.
    pub fn config_json_sources(&self) -> Result<ConfigJsonSources, ApplicationError> {
        let configuration = self
            .runtime_configuration
            .runtime_configuration()
            .map_err(|error| ApplicationError::internal_error(&error))?;
        let file = self
            .runtime_config_settings
            .read_file_settings(agena_runtime::ConfigSettingsGetInput::default())
            .map_err(|error| ApplicationError::internal_error(&error))?
            .value;
        let project_file = self
            .runtime_config_settings
            .read_project_file_settings(agena_runtime::ConfigSettingsGetInput::default())
            .map_err(|error| ApplicationError::internal_error(&error))?
            .value;
        let effective = configuration.effective_config;
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

    pub async fn config_json_sources_async(&self) -> Result<ConfigJsonSources, ApplicationError> {
        self.run_blocking(Self::config_json_sources).await?
    }

    /// Resolves the active configuration path for an editor/process effect.
    pub fn config_path(&self) -> Result<PathBuf, ApplicationError> {
        self.runtime_configuration
            .runtime_configuration()
            .map(|configuration| configuration.config_path)
            .map_err(|error| ApplicationError::internal_error(&error))
    }

    pub fn service(&self) -> &ApplicationService {
        &self.service
    }

    /// Returns the Git source-control projection shared by CLI, TUI, and
    /// transport consumers.
    pub async fn git_status(&self) -> Result<crate::dto::GitStatusResource, ApplicationError> {
        self.service.git_status().await
    }

    /// Initializes source control through the application use case while
    /// sharing status queries across clients.
    pub async fn git_init(&self) -> Result<crate::dto::GitStatusResource, ApplicationError> {
        self.service.git_init().await
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
        self.service.git_stage(request).await
    }

    /// Creates a commit through the application-owned Git use case.
    pub async fn git_commit(
        &self,
        request: crate::dto::GitCommitRequest,
    ) -> Result<crate::dto::GitCommitResource, ApplicationError> {
        match request.workspace_id {
            Some(workspace_id) => {
                self.service
                    .git_commit_for_workspace(workspace_id, request)
                    .await
            }
            None => self.service.git_commit(request).await,
        }
    }

    /// Creates a pull request through the application-owned Git use case.
    pub async fn git_create_pull_request(
        &self,
        request: crate::dto::GitPullRequestCreateRequest,
    ) -> Result<crate::dto::GitPullRequestResource, ApplicationError> {
        match request.workspace_id {
            Some(workspace_id) => {
                self.service
                    .git_create_pull_request_for_workspace(workspace_id, request)
                    .await
            }
            None => self.service.git_create_pull_request(request).await,
        }
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

    /// Read and business command capabilities; no mutable transcript aggregate.
    pub fn session_kernel(
        &self,
    ) -> Result<&agena_storage::kernel::SessionKernel, ApplicationError> {
        Ok(self.service.session_kernel())
    }

    /// Ephemeral non-part signals (activities/plugins/tool registry). These
    /// are best-effort observer notifications and are never persisted.
    pub fn live_signal_service(
        &self,
    ) -> Result<Arc<dyn agena_runtime::RuntimeLiveSignalService>, ApplicationError> {
        self.live_signals.clone().ok_or_else(|| {
            ApplicationError::service_unavailable("runtime live signals not initialised")
        })
    }

    /// Complete runtime status projection shared by the process-local Studio
    /// health surface and the WS/IPC protocol. REST and WS handlers call this
    /// instead of assembling the record themselves.
    pub async fn runtime_status_response(
        &self,
    ) -> Result<agena_api::resource::RuntimeStatusResponse, ApplicationError> {
        use agena_api::resource::{
            DefaultSelectionResource, ModelCatalogResponse, RuntimeAutomationResource,
            RuntimeCommandResource, RuntimeCommandsResource, RuntimeLspResource,
            RuntimeLspServerResource, RuntimeMcpResource, RuntimeMcpServerResource,
            RuntimeOperatorResource, RuntimePluginSurfaceResource, RuntimeStatusResponse,
            RuntimeTaskResource,
        };

        let status = self.runtime_status().runtime_status().await;
        let catalog = status.model_catalog;
        let selection = self.provider_catalog().default_selection();
        let default_selection = (!selection.is_empty()).then_some(DefaultSelectionResource {
            provider: selection.provider,
            adapter: selection.adapter,
            model: selection.model,
            thinking_mode: selection.thinking_mode,
            speed_mode: selection.speed_mode,
            verbosity: selection.verbosity,
        });
        let model_catalog = ModelCatalogResponse {
            refreshing: status.model_catalog_refreshing,
            last_refresh_at: catalog.last_refresh_at,
            last_successful_source: catalog
                .last_successful_source
                .map(model_catalog_source_kind_from_domain),
            last_failure: catalog.last_failure.map(Into::into),
            model_count: catalog.models.len(),
        };
        let background_tasks = status
            .background_tasks
            .into_iter()
            .map(crate::dto::runtime_background_task_resource)
            .collect();

        let mcp = RuntimeMcpResource {
            server_count: status.mcp.servers.len(),
            tool_count: status
                .mcp
                .servers
                .iter()
                .map(|server| server.tool_count)
                .sum(),
            servers: status
                .mcp
                .servers
                .into_iter()
                .map(|server| RuntimeMcpServerResource {
                    name: server.name,
                    tool_count: server.tool_count,
                })
                .collect(),
        };
        let lsp = RuntimeLspResource {
            server_count: status.lsp.servers.len(),
            diagnostics_count: status.lsp.diagnostics_count,
            files_with_diagnostics: status.lsp.files_with_diagnostics,
            servers: status
                .lsp
                .servers
                .into_iter()
                .map(|server| RuntimeLspServerResource {
                    name: server.name,
                    command: server.command,
                    file_extensions: server.file_extensions,
                    root_markers: server.root_markers,
                })
                .collect(),
        };
        let commands = RuntimeCommandsResource {
            client_count: status
                .commands
                .commands
                .iter()
                .filter(|command| command.target == "client")
                .count(),
            served_count: status
                .commands
                .commands
                .iter()
                .filter(|command| command.target != "client")
                .count(),
            commands: status
                .commands
                .commands
                .into_iter()
                .map(|command| RuntimeCommandResource {
                    name: command.name,
                    slash: command.slash,
                    description: command.description,
                    aliases: command.aliases,
                    group: command.group,
                    category: command.category,
                    plugin_id: command.plugin_id,
                    target: command.target,
                })
                .collect(),
        };
        let mut jobs = status
            .scheduled_jobs
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        crate::service::sort_jobs_for_display(&mut jobs);
        let automation = RuntimeAutomationResource {
            enabled: status.automation_available,
            job_count: jobs.len(),
            recent_jobs: jobs
                .into_iter()
                .take(10)
                .map(crate::service::scheduled_job_resource)
                .collect(),
        };
        Ok(RuntimeStatusResponse {
            generation: status.generation,
            loaded_at: status.loaded_at,
            workspace_root: status.workspace_root.display().to_string(),
            config_path: status.config_path.display().to_string(),
            config_found: status.config_found,
            provider_ids: status.provider_ids,
            plugin_count: status.plugin_count,
            session_runtime_available: status.session_runtime_available,
            watch_paths: status
                .watch_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            reload: RuntimeTaskResource {
                enabled: status.reload_enabled,
                interval_secs: status.reload_interval_secs,
            },
            session_gc: RuntimeTaskResource {
                enabled: status.session_gc_enabled,
                interval_secs: status.session_gc_interval_secs,
            },
            model_catalog: Some(model_catalog),
            default_selection,
            background_tasks,
            automation,
            operator: RuntimeOperatorResource {
                mcp,
                lsp,
                agent_id: status.agent_id,
                commands,
                plugins: RuntimePluginSurfaceResource {
                    catalog: plugin_surface_catalog_resource_from_domain(
                        status.plugin_surface_catalog,
                    ),
                    tool_registry_generation: status.tool_registry_generation,
                    tool_registry_last_event: status.tool_registry_last_event,
                },
            },
        })
    }
}

impl Application {
    async fn reload_after_authentication_change(&self) -> Result<(), ApplicationError> {
        self.runtime_control
            .reload()
            .await
            .map(|_| ())
            .map_err(ApplicationError::from)
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

fn model_catalog_search_json<T: serde::Serialize>(value: &T, field: &str) -> String {
    match serde_json::to_string(value) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                model_catalog_field = field,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    format!("serialize model catalog {field} for search indexing"),
                    &error,
                ),
                "model catalog search text omitted an unserializable field"
            );
            String::new()
        }
    }
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
                    .map(|value| model_catalog_search_json(value, "thinking mode"))
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
                model_catalog_search_json(&mode.request_override, "speed-mode request override"),
                model_catalog_search_json(&mode.adapter_overrides, "speed-mode adapter overrides"),
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

pub fn model_catalog_source_kind_from_domain(
    value: agena_provider::ModelCatalogSnapshotSourceKind,
) -> agena_api::resource::ModelCatalogSourceKind {
    match value {
        agena_provider::ModelCatalogSnapshotSourceKind::Generated => {
            agena_api::resource::ModelCatalogSourceKind::Generated
        }
        agena_provider::ModelCatalogSnapshotSourceKind::Cache => {
            agena_api::resource::ModelCatalogSourceKind::Cache
        }
    }
}

fn plugin_surface_catalog_resource_from_domain(
    value: agena_plugin_host::PluginSurfaceCatalog,
) -> agena_api::resource::PluginSurfaceCatalogResource {
    use agena_api::resource::{
        PluginCommandResource, PluginDisplayContributionResource, PluginSurfaceCatalogResource,
        PluginTerminalSurfaceCatalogResource, PluginThemeColorsResource,
        PluginThemePaletteResource,
    };

    PluginSurfaceCatalogResource {
        commands: value
            .commands
            .into_iter()
            .map(|item| PluginCommandResource {
                plugin_id: item.plugin_id.to_string(),
                accepts_empty_input: item.accepts_empty_input,
                default_input: item.default_input,
                command: item.command,
            })
            .collect(),
        terminal: PluginTerminalSurfaceCatalogResource {
            display: value
                .terminal
                .display
                .into_iter()
                .map(|item| PluginDisplayContributionResource {
                    plugin_id: item.plugin_id.to_string(),
                    id: item.contribution.id,
                    kind: item.contribution.kind,
                    priority: item.contribution.priority,
                    content: item.contribution.content,
                })
                .collect(),
            themes: value
                .terminal
                .themes
                .into_iter()
                .map(|theme| PluginThemePaletteResource {
                    id: theme.id,
                    plugin_id: theme.plugin_id.to_string(),
                    display_name: theme.display_name,
                    colors: PluginThemeColorsResource {
                        muted: theme.colors.muted.map(|color| color.as_str().to_owned()),
                        accent: theme.colors.accent.map(|color| color.as_str().to_owned()),
                        info: theme.colors.info.map(|color| color.as_str().to_owned()),
                        success: theme.colors.success.map(|color| color.as_str().to_owned()),
                        warning: theme.colors.warning.map(|color| color.as_str().to_owned()),
                        danger: theme.colors.danger.map(|color| color.as_str().to_owned()),
                        special: theme.colors.special.map(|color| color.as_str().to_owned()),
                        selection_fg: theme
                            .colors
                            .selection_fg
                            .map(|color| color.as_str().to_owned()),
                        selection_bg: theme
                            .colors
                            .selection_bg
                            .map(|color| color.as_str().to_owned()),
                    },
                })
                .collect(),
        },
    }
}
