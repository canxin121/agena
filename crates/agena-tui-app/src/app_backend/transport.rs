//! Cloneable transport boundary between the TUI state machine and the owner
//! of session execution.
//!
//! The TUI is a pure HTTP client. [`TuiBackend`] contains only the public API
//! client plus the server workspace identity; it never owns a Runtime,
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
        SessionOverviewResource, SessionResource, SessionTranscriptPart, UserInputReply,
    },
};
use agena_client::{AgenaClient, SubscriptionEvent};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use tokio::sync::mpsc;

use super::{LiveEvent, SessionRefresh};

pub(crate) const SESSION_TRANSCRIPT_PAGE_SIZE: u64 = 3;

#[derive(Debug, Clone)]
pub(crate) struct SessionTranscriptPage {
    pub parts: Vec<SessionTranscriptPart>,
    pub folds: Vec<agena_api::live::SessionTranscriptFoldResource>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionStateWithTranscriptPage {
    pub execution: SessionExecutionResource,
    pub page: SessionTranscriptPage,
}

pub(crate) fn transcript_part_from_resource(
    part: agena_api::live::PartResource,
) -> SessionTranscriptPart {
    SessionTranscriptPart {
        part_id: part.part_id,
        kind: part.kind,
        role: part.role,
        state: part.state,
        content: part.content,
        summary: part.summary,
        created_at_ms: part.created_at_ms,
        parent_part_id: part.parent_part_id,
        run_id: part.run_id,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendMode {
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionSubscriptionDispatch {
    Ignore,
    RefreshPluginRuntime,
    Emit { refresh_plugin_presentation: bool },
}

fn classify_session_subscription_event(
    event: &SubscriptionEvent,
    session_id: i64,
) -> SessionSubscriptionDispatch {
    match event {
        SubscriptionEvent::SessionChanged(change) => {
            if change.session_id() != session_id {
                return SessionSubscriptionDispatch::Ignore;
            }
            let refresh_plugin_presentation = matches!(
                change,
                agena_api::live::SessionChangeResource::PartAdded { part, .. }
                    | agena_api::live::SessionChangeResource::PartUpdated { part, .. }
                    if part.kind == "run"
            );
            SessionSubscriptionDispatch::Emit {
                refresh_plugin_presentation,
            }
        }
        SubscriptionEvent::RuntimeSignal(signal) => match signal.session_id {
            None if signal.kind == "tool_registry_changed" => {
                SessionSubscriptionDispatch::RefreshPluginRuntime
            }
            None => SessionSubscriptionDispatch::Ignore,
            Some(signal_session_id) if signal_session_id != session_id => {
                SessionSubscriptionDispatch::Ignore
            }
            Some(_) => SessionSubscriptionDispatch::Emit {
                refresh_plugin_presentation: signal.kind == "plugin",
            },
        },
        SubscriptionEvent::Lagged(_) => SessionSubscriptionDispatch::Emit {
            refresh_plugin_presentation: true,
        },
    }
}

#[derive(Clone)]
pub struct TuiBackend {
    inner: Arc<RemoteBackend>,
    /// Server-owned workspace path used only as a resource identity.
    workspace_root: Arc<PathBuf>,
    /// Empty client-local confinement root for media that has not been
    /// fetched through the authenticated workspace API.
    media_workspace: Arc<tempfile::TempDir>,
}

struct RemoteBackend {
    client: AgenaClient,
    workspace_id: i64,
    providers: tokio::sync::RwLock<Vec<ProviderSummaryResource>>,
    models: tokio::sync::RwLock<HashMap<String, Vec<agena_domain::Model>>>,
    configured_provider_adapter_models: tokio::sync::RwLock<
        HashMap<String, Vec<agena_api::resource::ProviderAdapterModelsResource>>,
    >,
    provider_drafts: tokio::sync::RwLock<
        HashMap<String, agena_application::provider_studio::ProviderConfigDraft>,
    >,
    /// Cached configuration-source read model assembled over HTTP. Loaded at
    /// connect and refreshed before settings-studio rebuilds; settings reads
    /// are synchronous in the TUI event loop.
    config_sources: tokio::sync::RwLock<Option<agena_application::dto::ConfigJsonSources>>,
    /// Authoritative runtime projection, including the fully resolved default
    /// provider/model selection after provider-level defaults are applied.
    runtime_status: tokio::sync::RwLock<Option<agena_api::resource::RuntimeStatusResponse>>,
    /// Cached control projection for Agena's own HTTP MCP surface.
    mcp_server_control: tokio::sync::RwLock<Option<serde_json::Value>>,
    /// Cached plugin UI catalog (display contributions, theme palettes, slash
    /// commands) fetched from the server. Plugin reads are synchronous in the
    /// TUI event loop.
    plugin_catalog: tokio::sync::RwLock<Option<agena_plugin_host::PluginUiCatalog>>,
    /// Cached plugin statuses fetched from the server.
    plugin_statuses: tokio::sync::RwLock<Vec<agena_plugin_host::status::PluginStatus>>,
    /// Cached plugin details and logs used by the synchronous workbench model.
    plugin_inspects: tokio::sync::RwLock<HashMap<String, agena_plugin_host::PluginInspect>>,
    plugin_logs: tokio::sync::RwLock<HashMap<String, Vec<agena_plugin_host::PluginLogRecord>>>,
    /// Plugin-adjacent presentation metadata returned with the UI catalog.
    permission_tools:
        tokio::sync::RwLock<Vec<agena_application::dto::PermissionToolCatalogResource>>,
    plugin_notifications: tokio::sync::RwLock<Vec<agena_plugin_host::HostNotification>>,
    activity_kinds: tokio::sync::RwLock<Vec<agena_domain::ActivityKind>>,
    /// Server-owned workspace inventory used by synchronous pickers/search.
    workspace_files: tokio::sync::RwLock<Option<agena_application::dto::WorkspaceFileTreeResource>>,
    workspace_file_index: tokio::sync::RwLock<Vec<PathBuf>>,
    /// Non-ignore-aware, shallow directory pages for path browsers. Keeping
    /// this separate prevents ignored build/config paths from disappearing
    /// while the workspace-wide mention index remains ignore-aware. Writes
    /// are single in-memory inserts, so a synchronous lock lets the TUI read
    /// authoritative pages instead of treating transient async lock
    /// contention as an empty directory.
    workspace_directory_cache:
        std::sync::RwLock<HashMap<String, Vec<agena_application::dto::WorkspaceFileNode>>>,
    workspace_image_data_urls: tokio::sync::RwLock<HashMap<String, String>>,
    /// Server-side credential environment metadata.
    aws_profiles: tokio::sync::RwLock<Vec<String>>,
    /// Cached model-catalog page fetched from the server. The settings studio
    /// reads model counts synchronously inside the TUI event loop.
    model_catalog: tokio::sync::RwLock<Option<agena_application::dto::ModelCatalogListResponse>>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkspacePathMetadata {
    pub(crate) is_directory: bool,
    pub(crate) size: Option<u64>,
}

fn find_workspace_file_node<'a>(
    nodes: &'a [agena_application::dto::WorkspaceFileNode],
    relative: &Path,
) -> Option<&'a agena_application::dto::WorkspaceFileNode> {
    let relative = relative.to_string_lossy().replace('\\', "/");
    for node in nodes {
        if node.path == relative {
            return Some(node);
        }
        if let Some(found) = find_workspace_file_node(&node.children, Path::new(&relative)) {
            return Some(found);
        }
    }
    None
}

fn collect_workspace_file_paths(
    nodes: &[agena_application::dto::WorkspaceFileNode],
    output: &mut Vec<PathBuf>,
) {
    for node in nodes {
        if node.kind == agena_application::dto::WorkspaceFileKind::File {
            output.push(PathBuf::from(&node.path));
        }
        collect_workspace_file_paths(&node.children, output);
    }
}

fn workspace_image_references(value: &serde_json::Value, output: &mut Vec<(String, String)>) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                workspace_image_references(value, output);
            }
        }
        serde_json::Value::Object(object) => {
            let is_image = object
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "image");
            if is_image
                && let Some(source) = object.get("source").and_then(serde_json::Value::as_object)
                && source
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|source| source == "local_path")
                && let Some(path) = source.get("path").and_then(serde_json::Value::as_str)
            {
                let mime = object
                    .get("mime")
                    .and_then(serde_json::Value::as_str)
                    .filter(|mime| mime.starts_with("image/"))
                    .map(str::to_owned)
                    .unwrap_or_else(|| image_mime_from_path(path));
                output.push((path.to_owned(), mime));
            }
            for value in object.values() {
                workspace_image_references(value, output);
            }
        }
        _ => {}
    }
}

fn replace_workspace_image_references(
    value: &mut serde_json::Value,
    images: &HashMap<String, String>,
) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                replace_workspace_image_references(value, images);
            }
        }
        serde_json::Value::Object(object) => {
            let replacement = object
                .get("source")
                .and_then(serde_json::Value::as_object)
                .filter(|source| {
                    source
                        .get("source")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|source| source == "local_path")
                })
                .and_then(|source| source.get("path"))
                .and_then(serde_json::Value::as_str)
                .and_then(|path| images.get(path))
                .cloned();
            if let Some(url) = replacement {
                object.insert(
                    "source".to_owned(),
                    serde_json::json!({ "source": "data_url", "url": url }),
                );
            }
            for value in object.values_mut() {
                replace_workspace_image_references(value, images);
            }
        }
        _ => {}
    }
}

fn image_mime_from_path(path: &str) -> String {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        _ => "image/png",
    }
    .to_owned()
}

impl TuiBackend {
    /// Connect to and validate a server, then resolve the local
    /// workspace path into the server's public workspace identity.
    pub async fn connect_remote(
        server_url: impl AsRef<str>,
        workspace_root: PathBuf,
    ) -> Result<Self> {
        Self::connect_remote_authenticated(server_url, Some(workspace_root), None, None).await
    }

    pub async fn connect_remote_authenticated(
        server_url: impl AsRef<str>,
        workspace_root: Option<PathBuf>,
        server_token: Option<&str>,
        server_password: Option<&str>,
    ) -> Result<Self> {
        let client =
            AgenaClient::connect_server(server_url.as_ref(), server_token, server_password)
                .await
                .context("server readiness/authentication handshake failed")?;
        let runtime_status = client
            .runtime_status()
            .await
            .context("failed to load the server runtime status")?;
        let workspace_root = match workspace_root {
            Some(workspace_root) => workspace_root,
            None => PathBuf::from(runtime_status.workspace_root.as_str()),
        };
        let workspace = client
            .command(Command::ResolveWorkspace(
                agena_api::commands::ResolveWorkspaceParams {
                    path: workspace_root.to_string_lossy().into_owned(),
                    create_if_missing: true,
                },
            ))
            .await
            .context("failed to resolve the TUI workspace through the server")?;
        let CommandResult::Workspace(workspace) = workspace else {
            bail!("server returned the wrong result while resolving the workspace");
        };
        let workspace_root = PathBuf::from(workspace.path.as_str());
        let providers = match client.query(Query::ListProviders).await? {
            QueryResult::Providers(providers) => providers,
            _ => bail!("server returned the wrong provider-list result"),
        };
        let mut models = HashMap::new();
        let mut configured_provider_adapter_models = HashMap::new();
        for provider in &providers {
            let response = client
                .configured_provider_models(provider.provider_id.as_str())
                .await
                .with_context(|| {
                    format!(
                        "failed to load models for provider {} from the server",
                        provider.provider_id
                    )
                })?;
            let resources = response.models;
            let provider_models = resources
                .iter()
                .cloned()
                .map(provider_model_from_resource)
                .collect::<Result<Vec<_>>>()
                .with_context(|| {
                    format!(
                        "provider {} returned model metadata incompatible with this TUI",
                        provider.provider_id
                    )
                })?;
            models.insert(provider.provider_id.clone(), provider_models);
            let configured = client
                .configured_provider_adapter_models(provider.provider_id.as_str())
                .await
                .with_context(|| {
                    format!(
                        "failed to load configured routes for provider {} from the server",
                        provider.provider_id
                    )
                })?;
            configured_provider_adapter_models.insert(provider.provider_id.clone(), configured);
        }
        let backend = Self {
            inner: Arc::new(RemoteBackend {
                client,
                workspace_id: workspace.id,
                providers: tokio::sync::RwLock::new(providers),
                models: tokio::sync::RwLock::new(models),
                configured_provider_adapter_models: tokio::sync::RwLock::new(
                    configured_provider_adapter_models,
                ),
                provider_drafts: Default::default(),
                config_sources: Default::default(),
                runtime_status: tokio::sync::RwLock::new(Some(runtime_status)),
                mcp_server_control: Default::default(),
                plugin_catalog: Default::default(),
                plugin_statuses: Default::default(),
                plugin_inspects: Default::default(),
                plugin_logs: Default::default(),
                permission_tools: Default::default(),
                plugin_notifications: Default::default(),
                activity_kinds: Default::default(),
                workspace_files: Default::default(),
                workspace_file_index: Default::default(),
                workspace_directory_cache: Default::default(),
                workspace_image_data_urls: Default::default(),
                aws_profiles: Default::default(),
                model_catalog: Default::default(),
            }),
            workspace_root: Arc::new(workspace_root),
            media_workspace: Arc::new(
                tempfile::tempdir().context("failed to prepare the TUI media cache")?,
            ),
        };
        // Config/plugin snapshots are presentation metadata; a failure here
        // must not prevent the client from connecting and driving sessions.
        let _ = backend.refresh_config_sources().await;
        let _ = backend.refresh_provider_drafts().await;
        let _ = backend.refresh_plugin_runtime_snapshot().await;
        let _ = backend.refresh_model_catalog_cache("", 0, 1).await;
        let _ = backend.refresh_workspace_file_tree().await;
        let _ = backend
            .refresh_workspace_directory(backend.workspace_root())
            .await;
        let _ = backend.refresh_aws_profiles().await;
        let _ = backend.refresh_mcp_server_control().await;
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

    pub(crate) fn workspace_id(&self) -> i64 {
        self.inner.workspace_id
    }

    pub(crate) fn media_workspace_root(&self) -> &Path {
        self.media_workspace.path()
    }

    /// Access to the HTTP client for operations ported to REST.
    pub(crate) fn client(&self) -> &AgenaClient {
        &self.inner.client
    }

    /// Read Agena's live MCP server control projection through the remote
    /// HTTP server. The TUI does not own an MCP runtime; this call always
    /// reaches the server process connected during startup.
    pub(crate) async fn mcp_server_control(&self) -> Result<serde_json::Value> {
        self.refresh_mcp_server_control().await
    }

    pub(crate) fn cached_mcp_server_control(&self) -> Option<serde_json::Value> {
        self.inner
            .mcp_server_control
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(crate) async fn refresh_mcp_server_control(&self) -> Result<serde_json::Value> {
        let value = self
            .inner
            .client
            .mcp_server_control()
            .await
            .context("failed to read the Agena MCP server control state")?;
        *self.inner.mcp_server_control.write().await = Some(value.clone());
        Ok(value)
    }

    pub(crate) async fn toggle_mcp_server(&self) -> Result<serde_json::Value> {
        let current = self.mcp_server_control().await?;
        let enabled = current
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let value = self
            .inner
            .client
            .update_mcp_server_control(!enabled, None)
            .await
            .context("failed to toggle the Agena MCP server")?;
        *self.inner.mcp_server_control.write().await = Some(value.clone());
        Ok(value)
    }

    pub(crate) async fn set_mcp_auth_enabled(
        &self,
        auth_enabled: bool,
    ) -> Result<serde_json::Value> {
        let value = self
            .inner
            .client
            .set_mcp_server_auth_enabled(auth_enabled)
            .await
            .context("failed to update Agena MCP authentication")?;
        *self.inner.mcp_server_control.write().await = Some(value.clone());
        Ok(value)
    }

    pub(crate) async fn set_mcp_public_url(
        &self,
        public_url: Option<String>,
    ) -> Result<serde_json::Value> {
        let current = self.mcp_server_control().await?;
        let enabled = current
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let value = self
            .inner
            .client
            .update_mcp_server_control(enabled, Some(public_url))
            .await
            .context("failed to update the Agena MCP public URL")?;
        *self.inner.mcp_server_control.write().await = Some(value.clone());
        Ok(value)
    }

    pub(crate) async fn set_mcp_oauth_password(&self, password: &str) -> Result<serde_json::Value> {
        let value = self
            .inner
            .client
            .set_mcp_server_oauth_password(password)
            .await
            .context("failed to set the Agena MCP OAuth password")?;
        *self.inner.mcp_server_control.write().await = Some(value.clone());
        Ok(value)
    }

    pub(crate) async fn clear_mcp_oauth_password(&self) -> Result<serde_json::Value> {
        let value = self
            .inner
            .client
            .clear_mcp_server_oauth_password()
            .await
            .context("failed to clear the Agena MCP OAuth password")?;
        *self.inner.mcp_server_control.write().await = Some(value.clone());
        Ok(value)
    }

    /// The cached configuration-source read model, if it has been loaded from
    /// the server. Synchronous because settings presentation is built inside
    /// the TUI event loop.
    pub(crate) fn config_sources(&self) -> Option<agena_application::dto::ConfigJsonSources> {
        self.inner
            .config_sources
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(crate) fn runtime_status(&self) -> Option<agena_api::resource::RuntimeStatusResponse> {
        self.inner
            .runtime_status
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(crate) async fn refresh_runtime_status_cache(
        &self,
    ) -> Result<agena_api::resource::RuntimeStatusResponse> {
        let status = self.inner.client.runtime_status().await?;
        *self.inner.runtime_status.write().await = Some(status.clone());
        Ok(status)
    }

    /// The resolved UI preferences projected from the server's effective
    /// configuration, for launching the terminal with the same
    /// theme/graphics/locale as an embedded runtime.
    pub fn tui_preferences(&self) -> agena_application::dto::TuiPreferencesResource {
        super::config::ui_configuration(self)
    }

    /// Refresh the cached configuration-source read model from the server:
    /// the global config file, the workspace config file, and the resolved
    /// effective document. The resolved-config endpoint wraps the effective
    /// value and layer metadata in `{ config, meta }`.
    pub(crate) async fn refresh_config_sources(
        &self,
    ) -> Result<agena_application::dto::ConfigJsonSources> {
        use agena_application::dto::ConfigJsonSources;
        let client = &self.inner.client;
        let global = client.settings_layer_value("global", "").await?;
        let workspace = client.settings_layer_value("workspace", "").await?;
        let resolved = client.resolved_config().await?;
        let (effective, applied_layers) = config_and_layers_from_resolved_response(resolved)?;
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
            applied_layers,
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

    async fn refresh_after_config_edit(&self, path: &str) -> Result<()> {
        self.refresh_config_sources().await?;
        let path = path.trim();
        let root = path.split('.').next().unwrap_or_default();
        match root {
            "providers" if !matches!(path, "providers.default" | "providers.default_selection") => {
                self.refresh_provider_runtime_snapshot().await?;
                return Ok(());
            }
            "plugins" => self.refresh_plugin_runtime_snapshot().await?,
            _ => {}
        }
        self.refresh_runtime_status_cache().await?;
        Ok(())
    }

    /// The cached plugin UI catalog, if loaded from the server.
    pub(crate) fn plugin_catalog(&self) -> Option<agena_plugin_host::PluginUiCatalog> {
        self.inner
            .plugin_catalog
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// The cached plugin statuses, if loaded from the server.
    pub(crate) fn plugin_statuses(&self) -> Vec<agena_plugin_host::status::PluginStatus> {
        self.inner
            .plugin_statuses
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Refresh the cached plugin snapshot from the server: statuses and the
    /// combined TUI/studio UI catalog.
    pub(crate) async fn refresh_plugin_runtime_snapshot(&self) -> Result<()> {
        self.refresh_plugin_presentation_snapshot().await?;
        let client = &self.inner.client;
        let statuses = plugin_statuses_from_response(client.plugin_statuses().await?)?;
        let mut inspects = self.inner.plugin_inspects.read().await.clone();
        let mut logs = self.inner.plugin_logs.read().await.clone();
        inspects.retain(|plugin_id, _| {
            statuses
                .iter()
                .any(|status| status.plugin_id.to_string() == *plugin_id)
        });
        logs.retain(|plugin_id, _| {
            statuses
                .iter()
                .any(|status| status.plugin_id.to_string() == *plugin_id)
        });
        let mut requests = tokio::task::JoinSet::new();
        for status in &statuses {
            let plugin_id = status.plugin_id.to_string();
            let client = client.clone();
            requests.spawn(async move {
                let (inspect, logs) = tokio::join!(
                    client.plugin_inspect(plugin_id.as_str()),
                    client.plugin_logs(plugin_id.as_str(), None, 200),
                );
                (plugin_id, inspect, logs)
            });
        }
        while let Some(result) = requests.join_next().await {
            let Ok((plugin_id, inspect, plugin_logs)) = result else {
                continue;
            };
            if let Ok(value) = inspect
                && let Some(value) = value.get("plugin").cloned()
                && let Ok(inspect) =
                    serde_json::from_value::<agena_plugin_host::PluginInspect>(value)
            {
                inspects.insert(plugin_id.clone(), inspect);
            }
            if let Ok(value) = plugin_logs
                && let Some(value) = value.get("logs").cloned()
                && let Ok(records) =
                    serde_json::from_value::<Vec<agena_plugin_host::PluginLogRecord>>(value)
            {
                logs.insert(plugin_id, records);
            }
        }
        *self.inner.plugin_statuses.write().await = statuses;
        *self.inner.plugin_inspects.write().await = inspects;
        *self.inner.plugin_logs.write().await = logs;
        Ok(())
    }

    /// Refresh only frame-consumed plugin presentation metadata. This is
    /// intentionally separate from workbench status/inspect/log reads so a
    /// run lifecycle signal costs one HTTP request instead of one per plugin.
    pub(crate) async fn refresh_plugin_presentation_snapshot(&self) -> Result<()> {
        let catalog_response = self.inner.client.plugin_ui_catalog().await?;
        let catalog = catalog_response
            .get("catalog")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let catalog = serde_json::from_value::<agena_plugin_host::PluginUiCatalog>(catalog)
            .context("the server returned an undecodable plugin UI catalog")?;
        let permission_tools = catalog_response
            .get("permission_tools")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .context("the server returned an undecodable permission tool catalog")?
            .unwrap_or_default();
        let notifications = catalog_response
            .get("notifications")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .context("the server returned undecodable plugin notifications")?
            .unwrap_or_default();
        let activity_kinds = catalog_response
            .get("activity_kinds")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .context("the server returned undecodable activity kinds")?
            .unwrap_or_else(agena_domain::builtin_activity_kinds);
        *self.inner.plugin_catalog.write().await = Some(catalog);
        *self.inner.permission_tools.write().await = permission_tools;
        *self.inner.plugin_notifications.write().await = notifications;
        *self.inner.activity_kinds.write().await = activity_kinds;
        Ok(())
    }

    pub(crate) fn permission_tools(
        &self,
    ) -> Vec<agena_application::dto::PermissionToolCatalogResource> {
        self.inner
            .permission_tools
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn plugin_notifications(&self) -> Vec<agena_plugin_host::HostNotification> {
        self.inner
            .plugin_notifications
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn activity_kinds(&self) -> Vec<agena_domain::ActivityKind> {
        self.inner
            .activity_kinds
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .filter(|kinds| !kinds.is_empty())
            .unwrap_or_else(agena_domain::builtin_activity_kinds)
    }

    pub(crate) async fn refresh_workspace_file_tree(&self) -> Result<()> {
        let value = self
            .inner
            .client
            .workspace_file_tree(self.inner.workspace_id, None, 64, 50_000, true)
            .await?;
        let tree: agena_application::dto::WorkspaceFileTreeResource = serde_json::from_value(value)
            .context("the server returned an undecodable workspace file tree")?;
        let mut index = Vec::new();
        collect_workspace_file_paths(&tree.entries, &mut index);
        index.sort();
        *self.inner.workspace_files.write().await = Some(tree);
        *self.inner.workspace_file_index.write().await = index;
        Ok(())
    }

    pub(crate) fn workspace_file_index(&self) -> Vec<PathBuf> {
        self.inner
            .workspace_file_index
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) async fn refresh_workspace_directory(&self, path: &Path) -> Result<()> {
        let relative = self.workspace_relative_path(path);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            bail!("workspace directory path is outside the active server workspace");
        }
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        let value = self
            .inner
            .client
            .workspace_file_tree(
                self.inner.workspace_id,
                (!relative_text.is_empty()).then_some(relative_text.as_str()),
                0,
                5_000,
                false,
            )
            .await?;
        let tree: agena_application::dto::WorkspaceFileTreeResource = serde_json::from_value(value)
            .context("the server returned an undecodable workspace directory")?;
        self.inner
            .workspace_directory_cache
            .write()
            .map_err(|_| anyhow::anyhow!("workspace directory cache lock poisoned"))?
            .insert(tree.path, tree.entries);
        Ok(())
    }

    pub(crate) fn workspace_directory_entries(
        &self,
        path: &Path,
    ) -> Vec<agena_application::dto::WorkspaceFileNode> {
        let relative = self.workspace_relative_path(path);
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if let Some(entries) = self
            .inner
            .workspace_directory_cache
            .read()
            .ok()
            .and_then(|guard| guard.get(relative_text.as_str()).cloned())
        {
            return entries;
        }
        let tree = self
            .inner
            .workspace_files
            .try_read()
            .ok()
            .and_then(|guard| guard.clone());
        let Some(tree) = tree else {
            return Vec::new();
        };
        if relative.as_os_str().is_empty() {
            return tree.entries;
        }
        find_workspace_file_node(&tree.entries, relative.as_path())
            .filter(|node| node.kind == agena_application::dto::WorkspaceFileKind::Directory)
            .map(|node| node.children.clone())
            .unwrap_or_default()
    }

    pub(crate) fn workspace_path_metadata(&self, path: &Path) -> Option<WorkspacePathMetadata> {
        let relative = self.workspace_relative_path(path);
        if relative.as_os_str().is_empty() {
            return Some(WorkspacePathMetadata {
                is_directory: true,
                size: None,
            });
        }
        let node = self
            .inner
            .workspace_files
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
            .and_then(|tree| find_workspace_file_node(&tree.entries, relative.as_path()).cloned())
            .or_else(|| {
                self.inner
                    .workspace_directory_cache
                    .read()
                    .ok()
                    .and_then(|guard| {
                        guard.values().find_map(|entries| {
                            find_workspace_file_node(entries, relative.as_path()).cloned()
                        })
                    })
            })?;
        match node.kind {
            agena_application::dto::WorkspaceFileKind::Directory => Some(WorkspacePathMetadata {
                is_directory: true,
                size: None,
            }),
            agena_application::dto::WorkspaceFileKind::File => Some(WorkspacePathMetadata {
                is_directory: false,
                size: node.size,
            }),
            agena_application::dto::WorkspaceFileKind::Symlink
            | agena_application::dto::WorkspaceFileKind::Other => None,
        }
    }

    fn workspace_relative_path(&self, path: &Path) -> PathBuf {
        let relative = if path.is_absolute() {
            path.strip_prefix(self.workspace_root())
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.to_path_buf())
        } else {
            path.to_path_buf()
        };
        // Cache keys come back from the server without trailing separators.
        // Rebuild from components so equivalent client inputs such as
        // `target`, `target/`, and `./target` address the same page.
        relative.components().collect()
    }

    pub(crate) async fn refresh_aws_profiles(&self) -> Result<()> {
        let value = self.inner.client.aws_profile_names().await?;
        let profiles = value
            .get("profiles")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .context("the server returned undecodable AWS profile names")?
            .unwrap_or_default();
        *self.inner.aws_profiles.write().await = profiles;
        Ok(())
    }

    pub(crate) fn aws_profiles(&self) -> Vec<String> {
        self.inner
            .aws_profiles
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) async fn download_workspace_file(&self, path: &str) -> Result<(String, Vec<u8>)> {
        self.inner
            .client
            .download_workspace_file(self.inner.workspace_id, path)
            .await
            .context("failed to download the server workspace file")
    }

    async fn download_server_image(
        &self,
        session_id: i64,
        path: &str,
    ) -> Result<(String, Vec<u8>)> {
        let requested = Path::new(path);
        let workspace_relative = if requested.is_absolute() {
            requested.strip_prefix(self.workspace_root()).ok()
        } else {
            Some(requested)
        };
        if let Some(relative) = workspace_relative {
            let relative = relative.to_string_lossy().replace('\\', "/");
            if let Ok(download) = self.download_workspace_file(relative.as_str()).await {
                return Ok(download);
            }
        }
        self.inner
            .client
            .download_session_media_file(session_id, path)
            .await
            .context("failed to download server-managed session media")
    }

    async fn localize_workspace_images(&self, execution: &mut SessionExecutionResource) {
        self.localize_workspace_image_parts(execution.session.id, &mut execution.parts)
            .await;
    }

    async fn localize_workspace_image_parts(
        &self,
        session_id: i64,
        parts: &mut [SessionTranscriptPart],
    ) {
        const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

        let mut references = Vec::new();
        for part in parts.iter() {
            workspace_image_references(&part.content, &mut references);
        }
        references.sort();
        references.dedup();
        if references.is_empty() {
            return;
        }

        let mut localized = self.inner.workspace_image_data_urls.read().await.clone();
        for (path, mime) in references {
            if localized.contains_key(path.as_str()) {
                continue;
            }
            if self
                .workspace_path_metadata(Path::new(path.as_str()))
                .and_then(|metadata| metadata.size)
                .is_some_and(|size| size > MAX_IMAGE_BYTES)
            {
                continue;
            }
            let Ok((_, bytes)) = self.download_server_image(session_id, path.as_str()).await else {
                continue;
            };
            if bytes.len() as u64 > MAX_IMAGE_BYTES {
                continue;
            }
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            localized.insert(path, format!("data:{mime};base64,{encoded}"));
        }
        *self.inner.workspace_image_data_urls.write().await = localized.clone();
        for part in parts.iter_mut() {
            replace_workspace_image_references(&mut part.content, &localized);
        }
    }

    async fn localized_execution(
        &self,
        mut execution: SessionExecutionResource,
    ) -> SessionExecutionResource {
        self.localize_workspace_images(&mut execution).await;
        execution
    }

    pub(crate) fn plugin_inspect(
        &self,
        plugin_id: &str,
    ) -> Option<agena_plugin_host::PluginInspect> {
        self.inner
            .plugin_inspects
            .try_read()
            .ok()
            .and_then(|guard| guard.get(plugin_id.trim()).cloned())
    }

    pub(crate) fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena_plugin_host::PluginLogRecord> {
        let mut records = self
            .inner
            .plugin_logs
            .try_read()
            .ok()
            .and_then(|guard| guard.get(plugin_id.trim()).cloned())
            .unwrap_or_default();
        if let Some(after_seq) = after_seq {
            records.retain(|record| record.seq > after_seq);
        }
        if records.len() > limit {
            records.drain(..records.len() - limit);
        }
        records
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
                    "failed to invoke plugin tool `{tool_name}` through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "plugin tool `{tool_name}` returned a response this TUI cannot decode: {error}"
            ))
        })?;
        let _ = self.refresh_plugin_presentation_snapshot().await;
        Ok(response)
    }

    /// Build the Provider Studio draft from the authenticated server snapshot.
    /// Draft reads stay synchronous because provider-workbench navigation runs
    /// in the terminal event loop; the cache is refreshed at connect and after
    /// every provider mutation.
    pub fn provider_config_draft(
        &self,
        provider_id: Option<&str>,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderConfigDraft,
        agena_application::ApplicationError,
    > {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            let mut draft = agena_application::provider_studio::ProviderConfigDraft::new_empty();
            draft.normalize_shape();
            return Ok(draft);
        };
        self.inner
            .provider_drafts
            .try_read()
            .ok()
            .and_then(|guard| guard.get(provider_id).cloned())
            .ok_or_else(|| {
                agena_application::ApplicationError::internal(format!(
                    "provider draft is not loaded: {provider_id}"
                ))
            })
    }

    pub async fn list_draft_provider_adapter_models(
        &self,
        draft: &agena_application::provider_studio::ProviderConfigDraft,
        adapter_ids: &[String],
    ) -> std::result::Result<
        agena_api::resource::ProviderAdapterModelsResponse,
        agena_application::ApplicationError,
    > {
        let value = self
            .client()
            .provider_studio_operation(
                "draft/models",
                serde_json::json!({
                    "draft": draft,
                    "adapter_ids": adapter_ids,
                }),
            )
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to list provider draft models through the server: {error}"
                ))
            })?;
        serde_json::from_value(value).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned undecodable provider draft models: {error}"
            ))
        })
    }

    pub async fn start_provider_draft_auth(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderDraftAuthActionResult,
        agena_application::provider_studio::ProviderDraftAuthError,
    > {
        self.provider_studio_auth_operation("auth/start", draft)
            .await
    }

    pub async fn continue_provider_draft_auth(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderDraftAuthActionResult,
        agena_application::provider_studio::ProviderDraftAuthError,
    > {
        self.provider_studio_auth_operation("auth/continue", draft)
            .await
    }

    async fn provider_studio_auth_operation(
        &self,
        operation: &str,
        draft: agena_application::provider_studio::ProviderConfigDraft,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderDraftAuthActionResult,
        agena_application::provider_studio::ProviderDraftAuthError,
    > {
        let value = self
            .client()
            .provider_studio_operation(operation, serde_json::json!({ "draft": draft }))
            .await
            .map_err(agena_application::provider_studio::ProviderDraftAuthError::other)?;
        match serde_json::from_value::<
            std::result::Result<
                agena_application::provider_studio::ProviderDraftAuthActionResult,
                agena_application::provider_studio::ProviderDraftAuthError,
            >,
        >(value.clone())
        {
            Ok(result) => result,
            Err(_) => match serde_json::from_value(value) {
                Ok(action) => Ok(action),
                Err(error) => {
                    Err(agena_application::provider_studio::ProviderDraftAuthError::other(error))
                }
            },
        }
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
                    "failed to list saved provider adapter models through the server: {error}"
                ))
            })
    }

    pub fn provider_model_draft_value(
        &self,
        draft: &agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        provider_model: Option<&agena_api::resource::ProviderModelResource>,
    ) -> std::result::Result<serde_json::Value, agena_application::ApplicationError> {
        let adapter_id = adapter_id.trim();
        let model_id = model_id.trim();
        if adapter_id.is_empty() || model_id.is_empty() {
            return Err(agena_application::ApplicationError::internal(
                "adapter id and model id are required",
            ));
        }
        if let Some(provider_id) = draft.source_provider_id.as_deref()
            && let Some(value) = self.config_sources().and_then(|sources| {
                sources
                    .file
                    .get("providers")?
                    .get(provider_id)?
                    .get("adapters")?
                    .get(adapter_id)?
                    .get("models")?
                    .get(model_id)
                    .cloned()
            })
            && !value.is_null()
        {
            return Ok(value);
        }
        Ok(
            agena_application::provider_studio::provider_model_draft_value_from_resource(
                model_id,
                provider_model,
            ),
        )
    }

    pub async fn save_provider_draft(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_model_lists: &[agena_api::resource::ProviderAdapterModelsResource],
        selected_adapter_ids: &[String],
        selected_model_keys: &std::collections::BTreeSet<String>,
        model_config_values: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        self.provider_studio_save_operation(
            "save",
            serde_json::json!({
                "draft": draft,
                "adapter_model_lists": adapter_model_lists,
                "selected_adapter_ids": selected_adapter_ids,
                "selected_model_keys": selected_model_keys,
                "model_config_values": model_config_values,
            }),
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
        self.provider_studio_save_operation(
            "save-adapter",
            serde_json::json!({
                "draft": draft,
                "adapter_models": adapter_models,
            }),
        )
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
        self.provider_studio_save_operation(
            "save-model",
            serde_json::json!({
                "draft": draft,
                "adapter_id": adapter_id,
                "model_id": model_id,
                "model_value": model_value,
            }),
        )
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
        self.provider_studio_save_operation(
            "delete-model",
            serde_json::json!({
                "draft": draft,
                "adapter_id": adapter_id,
                "model_id": model_id,
            }),
        )
        .await
    }

    pub async fn delete_provider(
        &self,
        provider_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        self.provider_studio_save_operation(
            "delete-provider",
            serde_json::json!({ "provider_id": provider_id }),
        )
        .await
    }

    pub async fn delete_provider_adapter(
        &self,
        draft: agena_application::provider_studio::ProviderConfigDraft,
        adapter_id: &str,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        self.provider_studio_save_operation(
            "delete-adapter",
            serde_json::json!({
                "draft": draft,
                "adapter_id": adapter_id,
            }),
        )
        .await
    }

    async fn provider_studio_save_operation(
        &self,
        operation: &str,
        body: serde_json::Value,
    ) -> std::result::Result<
        agena_application::provider_studio::ProviderStudioSaveResult,
        agena_application::provider_studio::ProviderStudioSaveError,
    > {
        let value = self
            .client()
            .provider_studio_operation(operation, body)
            .await
            .map_err(provider_studio_transport_error)?;
        let result = match serde_json::from_value::<
            std::result::Result<
                agena_application::provider_studio::ProviderStudioSaveResult,
                agena_application::provider_studio::ProviderStudioSaveError,
            >,
        >(value.clone())
        {
            Ok(result) => result,
            Err(_) => match serde_json::from_value(value) {
                Ok(result) => Ok(result),
                Err(error) => Err(provider_studio_transport_error(error)),
            },
        };
        if result.is_ok() {
            self.refresh_config_sources()
                .await
                .map_err(provider_studio_transport_error)?;
            self.refresh_provider_runtime_snapshot()
                .await
                .map_err(provider_studio_transport_error)?;
        }
        result
    }

    async fn refresh_provider_drafts(&self) -> Result<()> {
        let providers = self.inner.providers.read().await.clone();
        let mut drafts = self.inner.provider_drafts.read().await.clone();
        drafts.retain(|provider_id, _| {
            providers
                .iter()
                .any(|provider| provider.provider_id == *provider_id)
        });
        let mut failures = Vec::new();
        for provider in providers {
            let provider_id = provider.provider_id;
            let response = self
                .client()
                .provider_studio_draft(Some(provider_id.as_str()))
                .await;
            match response {
                Ok(value) => match serde_json::from_value(value) {
                    Ok(draft) => {
                        drafts.insert(provider_id, draft);
                    }
                    Err(error) => failures.push(format!(
                        "{provider_id}: the server returned an undecodable draft: {error}"
                    )),
                },
                Err(error) => failures.push(format!("{provider_id}: {error}")),
            }
        }
        *self.inner.provider_drafts.write().await = drafts;
        if failures.is_empty() {
            Ok(())
        } else {
            tracing::warn!(
                failed_provider_count = failures.len(),
                diagnostics = ?failures,
                "some Provider Studio drafts could not be refreshed"
            );
            bail!(
                "failed to refresh {} Provider Studio draft(s): {}",
                failures.len(),
                failures.join("; ")
            )
        }
    }

    pub(crate) async fn refresh_provider_runtime_snapshot(&self) -> Result<()> {
        let providers = match self.client().query(Query::ListProviders).await? {
            QueryResult::Providers(providers) => providers,
            _ => bail!("server returned the wrong provider-list result"),
        };
        let mut models = HashMap::new();
        let mut configured_adapter_models = HashMap::new();
        let mut drafts = self
            .inner
            .provider_drafts
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        drafts.retain(|provider_id, _| {
            providers
                .iter()
                .any(|provider| provider.provider_id == *provider_id)
        });
        for provider in &providers {
            let response = self
                .client()
                .configured_provider_models(provider.provider_id.as_str())
                .await?;
            let resources = response.models;
            let provider_models = resources
                .iter()
                .cloned()
                .map(provider_model_from_resource)
                .collect::<Result<Vec<_>>>()?;
            models.insert(provider.provider_id.clone(), provider_models);
            configured_adapter_models.insert(
                provider.provider_id.clone(),
                self.client()
                    .configured_provider_adapter_models(provider.provider_id.as_str())
                    .await?,
            );

            if let Ok(value) = self
                .client()
                .provider_studio_draft(Some(provider.provider_id.as_str()))
                .await
                && let Ok(draft) = serde_json::from_value(value)
            {
                drafts.insert(provider.provider_id.clone(), draft);
            }
        }
        *self.inner.providers.write().await = providers;
        *self.inner.models.write().await = models;
        *self.inner.configured_provider_adapter_models.write().await = configured_adapter_models;
        *self.inner.provider_drafts.write().await = drafts;
        self.refresh_runtime_status_cache().await?;
        Ok(())
    }

    /// Set a GLOBAL config file setting through the server, reloading the
    /// runtime when the edit requires it. The server validates the edit
    /// against the full composed configuration and owns the file write.
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
            .set_settings_layer_value("global", path.trim(), value, false, true)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to set global config setting `{path}` through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned an undecodable config edit response: {error}"
            ))
        })?;
        self.refresh_after_config_edit(path).await.map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the setting was saved but the TUI could not refresh its configuration snapshot: {error:#}"
            ))
        })?;
        Ok(response)
    }

    /// Delete a GLOBAL config file setting through the server, reloading the
    /// runtime when the edit requires it.
    pub async fn delete_config_setting(
        &self,
        path: &str,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        let response = self
            .client()
            .delete_settings_layer_value("global", path.trim(), false, true)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to delete global config setting `{path}` through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned an undecodable config edit response: {error}"
            ))
        })?;
        self.refresh_after_config_edit(path).await.map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the setting was deleted but the TUI could not refresh its configuration snapshot: {error:#}"
            ))
        })?;
        Ok(response)
    }

    /// Set a WORKSPACE-scoped config file setting through the server,
    /// reloading the runtime when the edit requires it.
    pub async fn set_workspace_config_setting(
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
                    "failed to set workspace config setting `{path}` through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned an undecodable config edit response: {error}"
            ))
        })?;
        self.refresh_after_config_edit(path).await.map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the workspace setting was saved but the TUI could not refresh its configuration snapshot: {error:#}"
            ))
        })?;
        Ok(response)
    }

    /// Delete a WORKSPACE-scoped config file setting through the server,
    /// reloading the runtime when the edit requires it.
    pub async fn delete_workspace_config_setting(
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
                    "failed to delete workspace config setting `{path}` through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned an undecodable config edit response: {error}"
            ))
        })?;
        self.refresh_after_config_edit(path).await.map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the workspace setting was deleted but the TUI could not refresh its configuration snapshot: {error:#}"
            ))
        })?;
        Ok(response)
    }

    /// Set the global default provider selection through the server. The
    /// server atomically patches `providers.default` and
    /// `providers.default_selection` on the global config and reloads the
    /// runtime, mirroring the canonical embedded
    /// `Application::set_provider_default_selection`.
    pub async fn set_provider_default_selection(
        &self,
        provider_id: &str,
        selection: serde_json::Value,
    ) -> std::result::Result<
        agena_runtime::ConfigSettingsEditResponse,
        agena_application::ApplicationError,
    > {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            return Err(agena_application::ApplicationError::internal(
                "provider id is required",
            ));
        }
        let changes = serde_json::json!({
            "default": provider_id,
            "default_selection": selection,
        });
        let response = self
            .client()
            .patch_settings("providers", changes, false, true)
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to set provider default selection through the server: {error}"
                ))
            })?;
        let response = serde_json::from_value(response).map_err(|error| {
            agena_application::ApplicationError::internal(format!(
                "the server returned an undecodable config edit response: {error}"
            ))
        })?;
        self.refresh_after_config_edit("providers.default_selection")
            .await
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "the default model was saved but the TUI could not refresh its configuration snapshot: {error:#}"
                ))
            })?;
        Ok(response)
    }

    /// Set a session's selected permission policy through the server.
    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: agena_domain::PermissionConfig,
    ) -> std::result::Result<SessionExecutionResource, agena_application::ApplicationError> {
        let resource: agena_api::resource::PermissionConfigResource =
            serde_json::from_value(serde_json::to_value(permission).map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to encode session permission: {error}"
                ))
            })?)
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
                    "failed to update session permission through the server: {error}"
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
            bail!("server returned the wrong session-tree result");
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
            bail!("server returned the wrong session-update result");
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
        Ok(self
            .get_session_state_with_transcript_page(session_id)
            .await?
            .execution)
    }

    pub(crate) async fn get_session_state_with_transcript_page(
        &self,
        session_id: i64,
    ) -> Result<SessionStateWithTranscriptPage> {
        // `/state` is an execution shell. Load only the newest bounded
        // collapsed transcript page separately; the server skips raw folded
        // activity before it crosses this transport boundary.
        let (mut execution, page_resource) = tokio::try_join!(
            self.client().get_session_state(session_id),
            self.client()
                .session_transcript_page(session_id, SESSION_TRANSCRIPT_PAGE_SIZE, None,),
        )?;
        let mut page = SessionTranscriptPage {
            parts: page_resource
                .parts
                .into_iter()
                .map(transcript_part_from_resource)
                .collect(),
            folds: page_resource.folds,
            next_cursor: page_resource.page.next_cursor,
            has_more: page_resource.page.has_more,
        };
        execution.parts = page.parts.clone();
        self.localize_workspace_images(&mut execution).await;
        page.parts = execution.parts.clone();
        Ok(SessionStateWithTranscriptPage { execution, page })
    }

    pub(crate) async fn list_session_transcript_page(
        &self,
        session_id: i64,
        limit: u64,
        cursor: &str,
    ) -> Result<SessionTranscriptPage> {
        let page_resource = self
            .client()
            .session_transcript_page(session_id, limit, Some(cursor))
            .await?;
        let mut page = SessionTranscriptPage {
            parts: page_resource
                .parts
                .into_iter()
                .map(transcript_part_from_resource)
                .collect(),
            folds: page_resource.folds,
            next_cursor: page_resource.page.next_cursor,
            has_more: page_resource.page.has_more,
        };
        self.localize_workspace_image_parts(session_id, &mut page.parts)
            .await;
        Ok(page)
    }

    pub(crate) async fn list_session_transcript_fold_page(
        &self,
        session_id: i64,
        limit: u64,
        cursor: &str,
    ) -> Result<SessionTranscriptPage> {
        let page_resource = self
            .client()
            .session_transcript_fold_page(session_id, limit, cursor)
            .await?;
        let mut page = SessionTranscriptPage {
            parts: page_resource
                .parts
                .into_iter()
                .map(transcript_part_from_resource)
                .collect(),
            folds: page_resource.folds,
            next_cursor: page_resource.page.next_cursor,
            has_more: page_resource.page.has_more,
        };
        self.localize_workspace_image_parts(session_id, &mut page.parts)
            .await;
        Ok(page)
    }

    pub async fn list_session_parts(
        &self,
        session_id: i64,
        limit: u64,
        cursor: Option<&str>,
    ) -> Result<agena_api::live::SessionPartsResource> {
        Ok(self
            .client()
            .session_parts_page(session_id, limit, cursor)
            .await?)
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
        let execution = self.get_session_state(session_id).await?;
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
        let execution = self
            .client()
            .submit_message(agena_api::commands::SubmitRunParams {
                session_id,
                options,
                document,
            })
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn continue_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        let execution = self.client().continue_run(session_id, options).await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn compact_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        let execution = self.client().compact_session(session_id, options).await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn update_session_selection(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource> {
        let execution = self
            .client()
            .update_session_selection(session_id, options)
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn cancel_run(
        &self,
        session_id: i64,
        execution_id: Option<agena_domain::ExecutionId>,
    ) -> Result<agena_domain::CancellationOutcome> {
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
        let execution = execution_result(result, "session rewind")?;
        Ok(self.localized_execution(execution).await)
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
        let execution = execution_result(result, "session fork")?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn reply_permission(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: PermissionReply,
    ) -> Result<SessionExecutionResource> {
        let execution = self
            .client()
            .reply_permission(agena_api::commands::ReplyPermissionParams {
                session_id,
                options,
                reply,
            })
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn reply_user_input(
        &self,
        session_id: i64,
        options: RunOptions,
        reply: UserInputReply,
    ) -> Result<SessionExecutionResource> {
        let execution = self
            .client()
            .reply_user_input(agena_api::commands::ReplyUserInputParams {
                session_id,
                options,
                reply,
            })
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn mark_interactive_request_presented(
        &self,
        session_id: i64,
        request_id: String,
    ) -> Result<SessionExecutionResource> {
        let execution = self
            .client()
            .mark_interactive_request_presented(session_id, request_id.as_str())
            .await?;
        Ok(self.localized_execution(execution).await)
    }

    pub async fn list_providers(&self) -> Result<Vec<ProviderSummaryResource>> {
        Ok(self.provider_summaries())
    }

    pub(crate) fn provider_summaries(&self) -> Vec<ProviderSummaryResource> {
        self.inner
            .providers
            .try_read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn list_local_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<agena_domain::Model>> {
        Ok(self
            .inner
            .models
            .try_read()
            .ok()
            .and_then(|guard| guard.get(provider_id.trim()).cloned())
            .unwrap_or_default())
    }

    pub(crate) fn configured_provider_adapter_models(
        &self,
        provider_id: &str,
    ) -> Vec<agena_api::resource::ProviderAdapterModelsResource> {
        self.inner
            .configured_provider_adapter_models
            .try_read()
            .ok()
            .and_then(|guard| guard.get(provider_id.trim()).cloned())
            .unwrap_or_default()
    }

    /// The cached model-catalog page, if one has been loaded from the server.
    /// Synchronous because the settings studio reads model counts inside the
    /// TUI event loop. An empty response is returned when nothing is cached
    /// yet, so sync consumers can render without blocking on HTTP.
    pub(crate) fn model_catalog(&self) -> agena_application::dto::ModelCatalogListResponse {
        self.inner
            .model_catalog
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_else(agena_application::dto::ModelCatalogListResponse::empty)
    }

    /// Fetch a model-catalog page from the server and cache it for
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
            .context("failed to load the model catalog from the server")?;
        let response = serde_json::from_value(value)
            .context("the server returned an undecodable model catalog")?;
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
        if let Some(selection) = self
            .runtime_status()
            .and_then(|status| status.default_selection)
            && let (Some(provider_id), Some(model_id)) =
                (selection.provider.as_deref(), selection.model.as_deref())
        {
            return match selection.adapter.as_deref() {
                Some(adapter_id) => {
                    agena_domain::ModelRef::try_new_with_adapter(provider_id, adapter_id, model_id)
                }
                None => agena_domain::ModelRef::try_new(provider_id, model_id),
            }
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "server default model reference is invalid: {error}"
                ))
            });
        }
        if let Some(sources) = self.config_sources()
            && let Some(selection) = sources
                .effective
                .get("providers")
                .and_then(|providers| providers.get("default_selection"))
                .and_then(|value| {
                    serde_json::from_value::<agena_domain::ModelSelectionConfig>(value.clone()).ok()
                })
            && let (Some(provider_id), Some(model_id)) =
                (selection.provider.as_deref(), selection.model.as_deref())
        {
            return match selection.adapter.as_deref() {
                Some(adapter_id) => {
                    agena_domain::ModelRef::try_new_with_adapter(provider_id, adapter_id, model_id)
                }
                None => agena_domain::ModelRef::try_new(provider_id, model_id),
            }
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "configured default model reference is invalid: {error}"
                ))
            });
        }

        let providers = self.provider_summaries();
        let configured_default = self.config_sources().and_then(|sources| {
            sources
                .effective
                .get("providers")?
                .get("default")?
                .as_str()
                .map(str::to_owned)
        });
        let provider = configured_default
            .as_deref()
            .and_then(|provider_id| {
                providers
                    .iter()
                    .find(|provider| provider.provider_id == provider_id)
            })
            .or_else(|| providers.first())
            .ok_or_else(|| {
                agena_application::ApplicationError::internal(
                    "server exposes no configured provider",
                )
            })?;
        let models = self
            .list_local_provider_models(provider.provider_id.as_str())
            .map_err(|error| {
                agena_application::ApplicationError::internal(format!(
                    "failed to read the cached provider models: {error}"
                ))
            })?;
        models
            .iter()
            .find(|model| model.id.as_ref() == provider.defaults.model)
            .or_else(|| models.first())
            .map(agena_domain::Model::reference)
            .ok_or_else(|| {
                agena_application::ApplicationError::internal("server exposes no configured model")
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
        let configured_modes = if request.model.is_none() {
            self.runtime_status()
                .and_then(|status| status.default_selection)
        } else {
            None
        };
        let thinking = configured_modes
            .as_ref()
            .and_then(|selection| selection.thinking_mode.clone())
            .or_else(|| {
                model
                    .thinking_modes
                    .iter()
                    .find(|mode| mode.is_default)
                    .or_else(|| model.thinking_modes.first())
                    .and_then(|mode| mode.selector().map(|selector| selector.into_owned()))
            });
        let speed = configured_modes
            .as_ref()
            .and_then(|selection| selection.speed_mode.clone())
            .or_else(|| {
                model
                    .speed_modes
                    .iter()
                    .find(|(_, mode)| mode.is_default)
                    .map(|(name, _)| name.clone())
            });
        (thinking, speed)
    }

    /// Start a session-scoped invalidation stream. Establishing the remote SSE
    /// stream happens inside the spawned task so the synchronous TUI event
    /// handler never blocks. Any event causes snapshot convergence; lag and
    /// transport closure are also surfaced as forced refreshes.
    pub fn subscribe_session_events(&self, session_id: i64) -> mpsc::Receiver<LiveEvent> {
        let backend = self.clone();
        let client = self.client().clone();
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            loop {
                // Subscribe before reading the snapshot. Global scope is
                // required for session-less runtime signals such as dynamic
                // tool-registry changes; session mutations are filtered below.
                let mut subscription = match client.stream_changes(agena_api::Scope::Global).await {
                    Ok(subscription) => subscription,
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
                let snapshot = match backend.get_session_state(session_id).await {
                    Ok(snapshot) => snapshot,
                    Err(_) => {
                        drop(subscription);
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
                        snapshot: Some(snapshot),
                        event: None,
                        force_refresh: false,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                while let Some(item) = subscription.recv().await {
                    let (force_refresh, refresh_plugin_presentation) = match item.as_ref() {
                        Err(_) => (true, true),
                        Ok(event) => match classify_session_subscription_event(event, session_id) {
                            SessionSubscriptionDispatch::Ignore => continue,
                            SessionSubscriptionDispatch::RefreshPluginRuntime => {
                                let _ = backend.refresh_plugin_runtime_snapshot().await;
                                continue;
                            }
                            SessionSubscriptionDispatch::Emit {
                                refresh_plugin_presentation,
                            } => (
                                matches!(event, SubscriptionEvent::Lagged(_)),
                                refresh_plugin_presentation,
                            ),
                        },
                    };
                    if refresh_plugin_presentation {
                        let _ = backend.refresh_plugin_presentation_snapshot().await;
                    }
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
            bail!("server returned the wrong session-list result");
        };
        Ok(page)
    }

    async fn get_session(&self, session_id: i64) -> Result<SessionResource> {
        let result = self
            .client()
            .query(Query::GetSession(GetSessionParams { session_id }))
            .await?;
        let QueryResult::Session(session) = result else {
            bail!("server returned the wrong session result");
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
                providers: Default::default(),
                models: Default::default(),
                configured_provider_adapter_models: Default::default(),
                provider_drafts: Default::default(),
                config_sources: Default::default(),
                runtime_status: Default::default(),
                mcp_server_control: Default::default(),
                plugin_catalog: Default::default(),
                plugin_statuses: Default::default(),
                plugin_inspects: Default::default(),
                plugin_logs: Default::default(),
                permission_tools: Default::default(),
                plugin_notifications: Default::default(),
                activity_kinds: Default::default(),
                workspace_files: Default::default(),
                workspace_file_index: Default::default(),
                workspace_directory_cache: Default::default(),
                workspace_image_data_urls: Default::default(),
                aws_profiles: Default::default(),
                model_catalog: Default::default(),
            }),
            workspace_root: Arc::new(std::env::temp_dir()),
            media_workspace: Arc::new(tempfile::tempdir().expect("mock media workspace")),
        }
    }
}

fn execution_result(result: CommandResult, operation: &str) -> Result<SessionExecutionResource> {
    let CommandResult::Execution(execution) = result else {
        bail!("server returned the wrong result for {operation}");
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

fn plugin_statuses_from_response(
    value: serde_json::Value,
) -> Result<Vec<agena_plugin_host::status::PluginStatus>> {
    serde_json::from_value(
        value
            .get("items")
            .cloned()
            .context("the plugin-status response has no `items` field")?,
    )
    .context("the server returned undecodable plugin statuses")
}

fn config_and_layers_from_resolved_response(
    value: serde_json::Value,
) -> Result<(serde_json::Value, Vec<String>)> {
    let effective = value
        .get("config")
        .cloned()
        .context("the resolved-config response has no `config` field")?;
    if !effective.is_object() {
        bail!("the resolved-config response contains a non-object `config` field");
    }
    let applied_layers = value
        .pointer("/meta/applied_layers")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|layer| {
            layer
                .get("description")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    Ok((effective, applied_layers))
}

fn provider_studio_transport_error(
    error: impl std::fmt::Display,
) -> agena_application::provider_studio::ProviderStudioSaveError {
    tracing::error!(diagnostic = %error, "Provider Studio transport failed");
    let failure = agena_failure::Failure::new(
        agena_failure::FailureCode::new("tui.provider_studio_transport_failed"),
        agena_failure::FailureCategory::DependencyUnavailable,
        agena_failure::FailureResponsibility::System,
        agena_failure::RetryDirective::AfterRefresh,
        agena_failure::RecoveryDirective::OpenSettings,
        agena_failure::FailureImpact::RequestRejected,
        agena_failure::UserPresentation::new(
            "tui.provider_studio_transport_failed",
            "Provider Studio could not synchronize with the server.",
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
    fn global_tool_registry_signal_bypasses_session_filter_for_plugin_refresh() {
        let event = SubscriptionEvent::RuntimeSignal(agena_api::live::RuntimeSignalResource {
            kind: "tool_registry_changed".to_owned(),
            session_id: None,
            payload: serde_json::json!({ "generation": 4 }),
        });
        assert_eq!(
            classify_session_subscription_event(&event, 99),
            SessionSubscriptionDispatch::RefreshPluginRuntime
        );
    }

    #[test]
    fn remote_backend_has_no_embedded_application_fallback() {
        let first_model = agena_domain::Model::new("first", "model-1");
        let selected_model = agena_domain::Model::new("selected", "model-2");
        let backend = TuiBackend {
            inner: Arc::new(RemoteBackend {
                client: AgenaClient::new("http://127.0.0.1:9").expect("client"),
                workspace_id: 7,
                providers: tokio::sync::RwLock::new(vec![
                    ProviderSummaryResource {
                        provider_id: "first".to_owned(),
                        defaults: agena_api::resource::ProviderDefaultsResource {
                            adapter: None,
                            model: "model-1".to_owned(),
                        },
                        adapters: Vec::new(),
                    },
                    ProviderSummaryResource {
                        provider_id: "selected".to_owned(),
                        defaults: agena_api::resource::ProviderDefaultsResource {
                            adapter: Some("responses".to_owned()),
                            model: "model-2".to_owned(),
                        },
                        adapters: Vec::new(),
                    },
                ]),
                models: tokio::sync::RwLock::new(HashMap::from([
                    ("first".to_owned(), vec![first_model]),
                    ("selected".to_owned(), vec![selected_model]),
                ])),
                configured_provider_adapter_models: Default::default(),
                provider_drafts: Default::default(),
                config_sources: Default::default(),
                runtime_status: Default::default(),
                mcp_server_control: Default::default(),
                plugin_catalog: Default::default(),
                plugin_statuses: Default::default(),
                plugin_inspects: Default::default(),
                plugin_logs: Default::default(),
                permission_tools: Default::default(),
                plugin_notifications: Default::default(),
                activity_kinds: Default::default(),
                workspace_files: Default::default(),
                workspace_file_index: Default::default(),
                workspace_directory_cache: Default::default(),
                workspace_image_data_urls: Default::default(),
                aws_profiles: Default::default(),
                model_catalog: Default::default(),
            }),
            workspace_root: Arc::new(PathBuf::from("/workspace")),
            media_workspace: Arc::new(tempfile::tempdir().expect("test media workspace")),
        };
        *backend
            .inner
            .config_sources
            .try_write()
            .expect("config cache lock") = Some(agena_application::dto::ConfigJsonSources {
            config_path: PathBuf::from("/config/agena.json"),
            config_found: true,
            project_config_path: PathBuf::from("/workspace/.agena/agena.json"),
            project_config_found: false,
            applied_layers: Vec::new(),
            file: serde_json::Value::Null,
            project_file: serde_json::Value::Null,
            effective: serde_json::json!({
                "providers": {
                    "default": "selected",
                    "default_selection": {
                        "provider": "selected",
                        "adapter": "responses",
                        "model": "model-2"
                    }
                }
            }),
        });

        assert_eq!(backend.mode(), BackendMode::Remote);
        let resolved = backend
            .resolved_model_for_run_options(&RunOptions::default())
            .expect("resolve cached remote default");
        assert_eq!(resolved.provider_id.as_ref(), "selected");
        assert_eq!(
            resolved.adapter_id.as_ref().map(AsRef::as_ref),
            Some("responses")
        );
        assert_eq!(resolved.model_id.as_ref(), "model-2");
    }

    #[test]
    fn provider_studio_uses_saved_routes_not_discovered_model_listing() {
        let backend = TuiBackend::remote_mock();
        backend
            .inner
            .models
            .try_write()
            .expect("model cache lock")
            .insert(
                "example".to_owned(),
                vec![
                    agena_domain::Model::new("example", "configured"),
                    agena_domain::Model::new("example", "discovered-only"),
                ],
            );
        backend
            .inner
            .configured_provider_adapter_models
            .try_write()
            .expect("configured route cache lock")
            .insert(
                "example".to_owned(),
                vec![agena_api::resource::ProviderAdapterModelsResource {
                    adapter_id: "responses".to_owned(),
                    enabled: false,
                    resolved_base_url: None,
                    models: vec![agena_api::resource::ProviderModelResource::configured(
                        "responses",
                        "configured",
                    )],
                    failure: None,
                }],
            );

        let routes = crate::app_backend::provider_mappings::configured_provider_model_routes(
            &backend,
            Some("example"),
        );
        assert!(
            routes.is_empty(),
            "disabled adapter routes are not selected"
        );
        let adapters = crate::app_backend::provider_mappings::configured_provider_adapter_models(
            &backend,
            Some("example"),
        );
        assert_eq!(adapters.len(), 1);
        assert!(!adapters[0].enabled);
        assert_eq!(adapters[0].models.len(), 1);
        assert_eq!(adapters[0].models[0].id, "configured");
    }

    #[test]
    fn plugin_status_cache_decodes_the_server_items_envelope() {
        let plugin_id =
            agena_plugin_host::PluginKey::new("agena", "settings").expect("valid plugin id");
        let status = agena_plugin_host::status::PluginStatus::initial(&plugin_id, "static");
        let decoded = plugin_statuses_from_response(serde_json::json!({
            "items": [status]
        }))
        .expect("decode plugin status response");

        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].plugin_id.to_string(), "agena.settings");
    }

    #[test]
    fn config_cache_decodes_the_resolved_config_envelope() {
        let (config, layers) = config_and_layers_from_resolved_response(serde_json::json!({
            "config": {
                "providers": {
                    "default": "selected",
                    "default_selection": {
                        "provider": "selected",
                        "adapter": "responses",
                        "model": "model-2"
                    }
                }
            },
            "meta": {
                "applied_layers": [
                    {"source": "default", "description": "built-in defaults"},
                    {"source": "file", "description": "file:/config/agena.json"}
                ]
            }
        }))
        .expect("decode resolved config response");

        assert_eq!(
            config.pointer("/providers/default_selection/model"),
            Some(&serde_json::json!("model-2"))
        );
        assert_eq!(layers, vec!["built-in defaults", "file:/config/agena.json"]);
    }

    #[test]
    fn provider_studio_can_create_a_fresh_draft_in_remote_mode() {
        let backend = TuiBackend::remote_mock();
        let draft = backend
            .provider_config_draft(None)
            .expect("new provider draft");

        assert!(draft.provider_id.is_empty());
        assert!(draft.source_provider_id.is_none());
    }

    #[test]
    fn server_workspace_tree_drives_file_index_and_metadata() {
        let backend = TuiBackend::remote_mock();
        let tree = agena_application::dto::WorkspaceFileTreeResource {
            workspace_id: 1,
            root: backend.workspace_root().display().to_string(),
            path: String::new(),
            entries: vec![agena_application::dto::WorkspaceFileNode {
                name: "src".to_owned(),
                path: "src".to_owned(),
                kind: agena_application::dto::WorkspaceFileKind::Directory,
                size: None,
                children: vec![agena_application::dto::WorkspaceFileNode {
                    name: "main.rs".to_owned(),
                    path: "src/main.rs".to_owned(),
                    kind: agena_application::dto::WorkspaceFileKind::File,
                    size: Some(42),
                    children: Vec::new(),
                }],
            }],
        };
        let mut index = Vec::new();
        collect_workspace_file_paths(&tree.entries, &mut index);
        *backend
            .inner
            .workspace_files
            .try_write()
            .expect("workspace tree lock") = Some(tree);
        *backend
            .inner
            .workspace_file_index
            .try_write()
            .expect("workspace index lock") = index;

        assert_eq!(
            backend.workspace_file_index(),
            [PathBuf::from("src/main.rs")]
        );
        assert!(
            backend
                .workspace_path_metadata(Path::new("src"))
                .is_some_and(|metadata| metadata.is_directory)
        );
        assert_eq!(
            backend
                .workspace_path_metadata(Path::new("src/main.rs"))
                .and_then(|metadata| metadata.size),
            Some(42)
        );
        assert_eq!(
            backend.workspace_directory_entries(Path::new("src")).len(),
            1
        );

        backend
            .inner
            .workspace_directory_cache
            .write()
            .expect("workspace directory cache lock")
            .insert(
                String::new(),
                vec![agena_application::dto::WorkspaceFileNode {
                    name: "target".to_owned(),
                    path: "target".to_owned(),
                    kind: agena_application::dto::WorkspaceFileKind::Directory,
                    size: None,
                    children: Vec::new(),
                }],
            );
        assert_eq!(
            backend.workspace_directory_entries(backend.workspace_root())[0].name,
            "target",
            "path browsing uses the non-ignore-aware shallow cache"
        );
        assert!(
            backend
                .workspace_path_metadata(Path::new("target"))
                .is_some_and(|metadata| metadata.is_directory)
        );
        backend
            .inner
            .workspace_directory_cache
            .write()
            .expect("workspace directory cache lock")
            .insert(
                "target".to_owned(),
                vec![agena_application::dto::WorkspaceFileNode {
                    name: "debug".to_owned(),
                    path: "target/debug".to_owned(),
                    kind: agena_application::dto::WorkspaceFileKind::Directory,
                    size: None,
                    children: Vec::new(),
                }],
            );
        assert_eq!(
            backend.workspace_directory_entries(Path::new("target/"))[0].name,
            "debug",
            "equivalent trailing-separator paths share a directory cache key"
        );
        assert_eq!(
            backend.workspace_file_index(),
            [PathBuf::from("src/main.rs")],
            "ignored paths do not enter the mention-search index"
        );
    }

    #[test]
    fn workspace_image_references_are_localized_only_in_the_client_projection() {
        let mut content = serde_json::json!({
            "attachments": [{
                "kind": "image",
                "mime": "image/png",
                "source": { "source": "local_path", "path": "images/result.png" }
            }]
        });
        let original = content.clone();
        let mut references = Vec::new();
        workspace_image_references(&content, &mut references);
        assert_eq!(
            references,
            [("images/result.png".to_owned(), "image/png".to_owned())]
        );

        replace_workspace_image_references(
            &mut content,
            &HashMap::from([(
                "images/result.png".to_owned(),
                "data:image/png;base64,iVBORw0KGgo=".to_owned(),
            )]),
        );
        assert_eq!(
            content.pointer("/attachments/0/source/source"),
            Some(&serde_json::json!("data_url"))
        );
        assert_eq!(
            original.pointer("/attachments/0/source/source"),
            Some(&serde_json::json!("local_path")),
            "the authoritative server projection must not be mutated"
        );
    }

    #[test]
    fn plugin_presentation_metadata_is_read_from_remote_caches() {
        let backend = TuiBackend::remote_mock();
        *backend
            .inner
            .permission_tools
            .try_write()
            .expect("permission tool lock") =
            vec![agena_application::dto::PermissionToolCatalogResource {
                name: "agena.shell.exec".to_owned(),
                summary: "Run a command".to_owned(),
                tags: vec!["shell".to_owned()],
            }];
        *backend
            .inner
            .activity_kinds
            .try_write()
            .expect("activity kind lock") = vec![agena_domain::ActivityKind {
            id: "example.trace".to_owned(),
            category: agena_domain::ActivityKindCategory::Plugin,
            label: "Trace".to_owned(),
        }];
        *backend
            .inner
            .plugin_notifications
            .try_write()
            .expect("notification lock") = vec![agena_plugin_host::HostNotification {
            plugin_id: "agena.terminal".to_owned(),
            body: "done".to_owned(),
            ..Default::default()
        }];

        assert_eq!(backend.permission_tools()[0].name, "agena.shell.exec");
        assert_eq!(backend.activity_kinds()[0].id, "example.trace");
        assert_eq!(backend.plugin_notifications()[0].body, "done");
    }
}
