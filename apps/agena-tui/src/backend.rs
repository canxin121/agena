use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, OnceLock},
};

use agena::event::{EventFilter, Scope, bus::SubscriptionItem};
use agena::permission::PermissionScope;
use agena::{
    config::{
        self, AnthropicProviderOptions, ConfigSettingsEditResponse, ConfigSettingsPatchInput,
        HttpProviderAdapterConfig, OpenAiApiModeConfig, OpenAiBackendConfig, OpenAiProviderOptions,
        ProcessEnvironment, ProviderAdapterDefinition, ProviderApiAuthConfig, ProviderAuthConfig,
        ResolvedProviderAdapterConfig, SharedGatewayEndpointLayout, SimpleHttpProviderOptions,
        StreamTransportMode, patch_file_settings, probe_provider_adapters,
    },
    event::{DomainEvent, EventKind},
    memory::MemoryStore,
    message::{
        AttachmentItem, AttachmentKind, AttachmentSource, EnterWorktreeToolInput,
        ExitWorktreeToolInput, PartContent, ToolInvocation, UserInputReply,
    },
    model::ModelRef,
    permission::PermissionReplyKind,
    provider::ProviderModel,
    runtime::AgenaRuntime,
    tool,
};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WorktreeCommandOutput {
    #[serde(default)]
    pub action: Option<String>,
    pub path: String,
    #[serde(default)]
    pub branch: Option<String>,
}

fn parse_worktree_payload(payload: Option<serde_json::Value>) -> Result<WorktreeCommandOutput> {
    let payload = payload.ok_or_else(|| anyhow!("worktree tool returned no payload"))?;
    serde_json::from_value(payload).map_err(|error| anyhow!(error.to_string()))
}
use agena_api::{
    commands::{
        Command as ApiCommand, CommandResult, ContinueRunParams, CreateSessionParams,
        ReplacePermissionRuleParams, ReplyPermissionParams, ReplyUserInputParams,
        RewindSessionParams, SubmitTurnParams, UpdateSessionParams, UpsertPermissionRuleParams,
    },
    pagination::PaginatedResponse,
    queries::{
        GetSessionParams, ListMessagesParams, ListPermissionRulesParams, ListSessionsParams, Query,
        QueryResult,
    },
    resource::{
        MessageResource, PartLoadMode, PermissionReply, PermissionRuleResource,
        ProviderAdapterSummaryResource, ProviderSummaryResource, RunOptions,
        SessionExecutionResource, SessionResource, WorkspaceResource,
    },
};
use agena_api_server::{
    dispatch,
    local_api::{
        ModelCatalogEntryKind, ModelCatalogEntryResource, ModelCatalogListResponse,
        ModelCatalogResponse as LocalModelCatalogResponse, ModelCatalogSourceKind,
        ProviderAdapterDiscoveryResource, ProviderAdapterDiscoveryResponse, normalize_limit,
    },
    state::AppState,
};
use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ignore::WalkBuilder;
use mime_guess::MimeGuess;
use sea_orm::DatabaseConnection;
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use tokio::sync::mpsc;

const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SessionRefresh {
    pub latest_event_seq: Option<i64>,
    pub event_count: usize,
    pub execution: Option<SessionExecutionResource>,
    pub latest_messages: Option<PaginatedResponse<MessageResource>>,
}

#[derive(Debug, Clone)]
struct GitStatusResource {
    workspace_root: String,
    git_available: bool,
    repo: bool,
    gh_available: bool,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
    staged_files: u64,
    unstaged_files: u64,
    untracked_files: u64,
    changed_files: u64,
    clean: bool,
    worktree_active_sessions: u64,
    worktree_managed_dirs: u64,
}

#[derive(Debug, Clone)]
pub struct InspectorRow {
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderDraftAuthKind {
    Api,
    Other(String),
}

impl ProviderDraftAuthKind {
    pub fn label(&self) -> &str {
        match self {
            Self::Api => "api",
            Self::Other(label) => label.as_str(),
        }
    }

    pub fn is_api(&self) -> bool {
        matches!(self, Self::Api)
    }
}

#[derive(Debug, Clone)]
pub struct ProviderConfigDraft {
    pub source_provider_id: Option<String>,
    pub provider_id: String,
    pub auth_kind: ProviderDraftAuthKind,
    pub base_url: String,
    pub api_key_env: String,
    pub api_key: String,
    pub default_adapter: String,
    pub default_model: String,
}

/// Push notification emitted by the unified bus for the active session.
/// Indicates whether the change requires reloading messages.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    /// True for events that materially change session state — the UI should
    /// trigger a `refresh_session` after handling.
    pub triggers_refresh: bool,
}

#[derive(Clone)]
pub struct Backend {
    runtime: AgenaRuntime,
    app_state: AppState,
    workspace_root: PathBuf,
    file_index: Arc<OnceLock<Vec<PathBuf>>>,
}

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
                registry
                    .get(provider_id.as_str())
                    .map(|provider| ProviderSummaryResource {
                        default_adapter: provider.default_adapter().map(ToString::to_string),
                        default_model: provider.default_model().to_string(),
                        adapters: Vec::new(),
                        provider_id,
                    })
            })
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        providers
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
                default_adapter: Some(provider.default_adapter.clone()),
                default_model: provider.default_model.clone(),
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
            })
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        providers
    }

    pub fn provider_config_draft(&self, provider_id: Option<&str>) -> Result<ProviderConfigDraft> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(ProviderConfigDraft {
                source_provider_id: None,
                provider_id: String::new(),
                auth_kind: ProviderDraftAuthKind::Api,
                base_url: String::new(),
                api_key_env: String::new(),
                api_key: String::new(),
                default_adapter: "openai".to_owned(),
                default_model: String::new(),
            });
        };

        let snapshot = self.runtime.current_snapshot();
        let provider = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow!("provider not found: {provider_id}"))?;

        let (auth_kind, base_url, api_key_env, api_key) = match &provider.auth {
            ProviderAuthConfig::Api(api) => (
                ProviderDraftAuthKind::Api,
                api.base_url.clone(),
                api.api_key_env.clone().unwrap_or_default(),
                api.api_key.clone().unwrap_or_default(),
            ),
            ProviderAuthConfig::None => (
                ProviderDraftAuthKind::Other("none".to_owned()),
                String::new(),
                String::new(),
                String::new(),
            ),
            ProviderAuthConfig::Credential(config) => (
                ProviderDraftAuthKind::Other(
                    format!("credential:{:?}", config.issuer).to_lowercase(),
                ),
                String::new(),
                String::new(),
                String::new(),
            ),
            ProviderAuthConfig::BedrockSigv4(sigv4) => (
                ProviderDraftAuthKind::Other("bedrock_sigv4".to_owned()),
                sigv4.base_url.clone(),
                String::new(),
                String::new(),
            ),
            ProviderAuthConfig::GoogleAdc(adc) => (
                ProviderDraftAuthKind::Other("google_adc".to_owned()),
                adc.base_url.clone(),
                String::new(),
                String::new(),
            ),
            ProviderAuthConfig::SapAiCore(config) => (
                ProviderDraftAuthKind::Other("sap_ai_core".to_owned()),
                config.api.base_url.clone(),
                String::new(),
                String::new(),
            ),
        };

        Ok(ProviderConfigDraft {
            source_provider_id: Some(provider_id.to_owned()),
            provider_id: provider_id.to_owned(),
            auth_kind,
            base_url,
            api_key_env,
            api_key,
            default_adapter: provider.default_adapter.clone(),
            default_model: provider.default_model.clone(),
        })
    }

    pub async fn list_provider_models(&self, provider_id: &str) -> Result<Vec<ProviderModel>> {
        self.runtime
            .current_snapshot()
            .list_provider_models(provider_id)
            .await
            .context("failed to list provider models")
    }

    pub fn list_model_catalog_entries(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<ModelCatalogListResponse> {
        let snapshot = self.runtime.current_snapshot();
        let catalog = snapshot.model_catalog_response();
        let summary = local_model_catalog_summary(&catalog);
        let entries = local_model_catalog_entry_resources(&catalog);
        let search = query.trim().to_lowercase();
        let available_origins = {
            let mut origins = entries
                .iter()
                .filter_map(|entry| {
                    let origin = entry.origin.clone().unwrap_or_default();
                    let trimmed = origin.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_owned())
                })
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            origins.sort();
            origins
        };
        let filtered = entries
            .into_iter()
            .filter(|entry| {
                search.is_empty() || local_model_catalog_entry_search_text(entry).contains(&search)
            })
            .collect::<Vec<_>>();
        let total = filtered.len();
        let limit = normalize_limit(Some(limit as u64)) as usize;
        let items = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        Ok(ModelCatalogListResponse {
            summary,
            total,
            offset,
            limit,
            available_origins,
            items,
        })
    }

    pub fn lookup_model_catalog_entries(
        &self,
        model_ids: &[String],
    ) -> Vec<ModelCatalogEntryResource> {
        let requested = model_ids
            .iter()
            .flat_map(|model_id| {
                let raw = model_id.trim().to_owned();
                if raw.is_empty() {
                    return Vec::new();
                }
                let canonical = agena::model_catalog::canonical_model_catalog_id(raw.as_str());
                if canonical.is_empty() || canonical == raw {
                    vec![raw]
                } else {
                    vec![raw, canonical]
                }
            })
            .collect::<std::collections::BTreeSet<_>>();
        let snapshot = self.runtime.current_snapshot();
        let catalog = snapshot.model_catalog_response();
        local_model_catalog_entry_resources(&catalog)
            .into_iter()
            .filter(|entry| requested.contains(entry.model_id.as_str()))
            .collect()
    }

    pub async fn refresh_model_catalog(&self) -> Result<()> {
        let snapshot = self.runtime.current_snapshot();
        let source_providers = snapshot.catalog_source_provider_registry();
        snapshot
            .model_catalog()
            .refresh_from_registry(
                source_providers.as_ref(),
                Some(snapshot.config_resolution()),
            )
            .await
            .context("failed to refresh model catalog")?;
        Ok(())
    }

    pub async fn discover_draft_provider_adapters(
        &self,
        draft: &ProviderConfigDraft,
    ) -> Result<ProviderAdapterDiscoveryResponse> {
        if !draft.auth_kind.is_api() {
            return Err(anyhow!(
                "draft discovery requires api auth; current auth is {}",
                draft.auth_kind.label()
            ));
        }
        let base_url = draft.base_url.trim();
        if base_url.is_empty() {
            return Err(anyhow!("adapter discovery requires a base URL"));
        }

        let auth = ProviderAuthConfig::Api(ProviderApiAuthConfig {
            base_url: base_url.to_owned(),
            endpoint_layout: SharedGatewayEndpointLayout::Auto,
            api_key: optional_string(draft.api_key.as_str()),
            api_key_env: optional_string(draft.api_key_env.as_str()),
        });
        let adapters = draft_discovery_adapters(&[])?;
        self.discover_provider_adapters_with_config(
            optional_non_empty(draft.provider_id.as_str()).unwrap_or("draft"),
            &auth,
            adapters,
        )
        .await
    }

    pub async fn discover_saved_provider_adapters(
        &self,
        provider_id: &str,
    ) -> Result<ProviderAdapterDiscoveryResponse> {
        let provider_id = provider_id.trim();
        let snapshot = self.runtime.current_snapshot();
        let resolved = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow!("provider not found: {provider_id}"))?;
        let adapters = saved_discovery_adapters(provider_id, resolved, &resolved.auth, &[])?;
        self.discover_provider_adapters_with_config(provider_id, &resolved.auth, adapters)
            .await
    }

    pub async fn save_provider_draft(
        &self,
        draft: ProviderConfigDraft,
        discoveries: &[ProviderAdapterDiscoveryResource],
        selected_adapter_ids: &[String],
    ) -> Result<String> {
        let provider_id = required_trimmed(draft.provider_id.as_str(), "provider_id")?;
        let default_adapter = required_trimmed(draft.default_adapter.as_str(), "default_adapter")?;
        let default_model = required_trimmed(draft.default_model.as_str(), "default_model")?;

        let catalog_entries = self.lookup_model_catalog_entries(
            &discoveries
                .iter()
                .flat_map(|discovery| discovery.models.iter().map(|model| model.id.to_string()))
                .chain(std::iter::once(default_model.to_owned()))
                .collect::<Vec<_>>(),
        );
        let selected = selected_adapter_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<std::collections::BTreeSet<_>>();

        let mut adapters = JsonMap::new();
        for discovery in discoveries {
            if !discovery.supported || !selected.contains(discovery.adapter_id.as_str()) {
                continue;
            }
            let matched_models = discovery
                .models
                .iter()
                .filter_map(|model| {
                    preferred_catalog_entry_for_model_id(&catalog_entries, model.id.as_str()).map(
                        |entry| {
                            (
                                model.id.to_string(),
                                catalog_entry_to_provider_model_value(&entry),
                            )
                        },
                    )
                })
                .collect::<JsonMap<_, _>>();
            let mut adapter_value = JsonMap::new();
            adapter_value.insert("enabled".to_owned(), JsonValue::Bool(true));
            if !matched_models.is_empty() {
                adapter_value.insert("models".to_owned(), JsonValue::Object(matched_models));
            }
            adapters.insert(
                discovery.adapter_id.clone(),
                JsonValue::Object(adapter_value),
            );
        }

        let default_model_value = provider_model_json_for_model_id(
            &catalog_entries,
            default_model,
            None::<&ProviderModel>,
        );
        adapters
            .entry(default_adapter.to_owned())
            .or_insert_with(|| json!({ "enabled": true }));
        ensure_provider_model_entry(
            adapters
                .get_mut(default_adapter)
                .expect("default adapter must exist"),
            default_model,
            default_model_value,
        )?;

        let provider_patch = build_provider_patch_value(
            &draft,
            default_adapter,
            default_model,
            JsonValue::Object(adapters),
            true,
        )?;
        self.patch_provider_settings(provider_id, provider_patch)
            .await?;
        Ok(format!(
            "Saved provider {provider_id} with default {default_adapter}/{default_model}."
        ))
    }

    pub async fn save_provider_adapter_matches(
        &self,
        draft: ProviderConfigDraft,
        discovery: ProviderAdapterDiscoveryResource,
    ) -> Result<String> {
        let provider_id = required_trimmed(draft.provider_id.as_str(), "provider_id")?;
        let adapter_id = required_trimmed(discovery.adapter_id.as_str(), "adapter_id")?;
        let catalog_entries = self.lookup_model_catalog_entries(
            &discovery
                .models
                .iter()
                .map(catalog_lookup_id_for_provider_model)
                .collect::<Vec<_>>(),
        );
        let matched_models = discovery
            .models
            .iter()
            .filter_map(|model| {
                preferred_catalog_entry_for_provider_model(&catalog_entries, model).map(|entry| {
                    (
                        model.id.to_string(),
                        catalog_entry_to_provider_model_value(&entry),
                    )
                })
            })
            .collect::<JsonMap<_, _>>();
        let provider_patch = build_provider_patch_value(
            &draft,
            optional_non_empty(draft.default_adapter.as_str()).unwrap_or(adapter_id),
            optional_non_empty(draft.default_model.as_str()).unwrap_or("default"),
            json!({
                adapter_id: {
                    "enabled": true,
                    "models": matched_models,
                }
            }),
            false,
        )?;
        self.patch_provider_settings(provider_id, provider_patch)
            .await?;
        Ok(format!(
            "Saved {provider_id}/{adapter_id} with {} catalog-matched model(s).",
            matched_models.len()
        ))
    }

    pub async fn save_provider_model(
        &self,
        draft: ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        provider_model: Option<ProviderModel>,
        set_default: bool,
    ) -> Result<String> {
        let provider_id = required_trimmed(draft.provider_id.as_str(), "provider_id")?;
        let adapter_id = required_trimmed(adapter_id, "adapter_id")?;
        let model_id = required_trimmed(model_id, "model_id")?;
        let catalog_entries =
            self.lookup_model_catalog_entries(&[catalog_lookup_id_for_model_id(model_id)]);
        let model_value =
            provider_model_json_for_model_id(&catalog_entries, model_id, provider_model.as_ref());
        let default_adapter = if set_default {
            adapter_id
        } else {
            optional_non_empty(draft.default_adapter.as_str()).unwrap_or(adapter_id)
        };
        let default_model = if set_default {
            model_id
        } else {
            optional_non_empty(draft.default_model.as_str()).unwrap_or(model_id)
        };
        let provider_patch = build_provider_patch_value(
            &draft,
            default_adapter,
            default_model,
            json!({
                adapter_id: {
                    "enabled": true,
                    "models": {
                        model_id: model_value,
                    }
                }
            }),
            set_default || draft.source_provider_id.is_none(),
        )?;
        self.patch_provider_settings(provider_id, provider_patch)
            .await?;
        Ok(format!("Saved {provider_id}/{adapter_id}/{model_id}."))
    }

    async fn discover_provider_adapters_with_config(
        &self,
        provider_id: &str,
        auth: &ProviderAuthConfig,
        adapters: std::collections::BTreeMap<String, ResolvedProviderAdapterConfig>,
    ) -> Result<ProviderAdapterDiscoveryResponse> {
        let client = agena::provider::ProviderRegistry::build_http_client(
            self.runtime
                .config_resolution()
                .config
                .provider_http_client_config(),
        )
        .context("failed to build provider discovery http client")?;
        let probes =
            probe_provider_adapters(provider_id, auth, &adapters, client, &ProcessEnvironment)
                .await;
        Ok(ProviderAdapterDiscoveryResponse {
            provider_id: provider_id.to_owned(),
            adapters: probes
                .into_iter()
                .map(|probe| ProviderAdapterDiscoveryResource {
                    adapter_id: probe.adapter_id,
                    enabled: probe.enabled,
                    supported: probe.supported,
                    resolved_base_url: probe.resolved_base_url,
                    models: probe.models,
                    error: probe.error,
                })
                .collect(),
        })
    }

    async fn patch_provider_settings(
        &self,
        provider_id: &str,
        provider_patch: JsonValue,
    ) -> Result<ConfigSettingsEditResponse> {
        let config_path = self.runtime.config_resolution().meta.config_path.clone();
        let response = patch_file_settings(
            config_path,
            ConfigSettingsPatchInput {
                path: Some("providers".to_owned()),
                changes: json!({
                    provider_id: provider_patch,
                }),
                dry_run: false,
                validate: true,
                reload: true,
            },
        )
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to patch provider settings")?;

        if response.reload_required {
            self.runtime
                .reload()
                .await
                .context("failed to reload runtime after provider settings change")?;
        }
        Ok(response)
    }

    pub async fn list_child_sessions(&self, parent_id: i64) -> Result<Vec<SessionResource>> {
        let workspace_id = self.current_workspace_id().await?;
        self.list_sessions_query(ListSessionsParams {
            cursor: None,
            limit: Some(200),
            workspace_id: Some(workspace_id),
            parent_id: Some(parent_id),
            roots: false,
            search: None,
        })
        .await
        .context("failed to list child sessions")
    }

    pub async fn get_session(&self, session_id: i64) -> Result<Option<SessionResource>> {
        match dispatch::dispatch_query(
            &self.app_state,
            Query::GetSession(GetSessionParams { session_id }),
        )
        .await
        {
            Ok(QueryResult::Session(session)) => Ok(Some(session)),
            Ok(other) => Err(anyhow!("unexpected query result: {:?}", other))
                .context("failed to fetch session"),
            Err(agena_api_server::error::ServerError::NotFound(_)) => Ok(None),
            Err(error) => Err(api_error(error).context("failed to fetch session")),
        }
    }

    pub async fn list_session_subtree(&self, session_id: i64) -> Result<Vec<SessionResource>> {
        let root = self.resolve_session_root(session_id).await?;
        let mut items = vec![root.clone()];
        let mut seen = HashSet::from([root.id]);
        let mut stack = vec![root.id];

        while let Some(parent_id) = stack.pop() {
            let children = self
                .list_sessions_query(ListSessionsParams {
                    cursor: None,
                    limit: Some(200),
                    workspace_id: Some(root.workspace_id),
                    parent_id: Some(parent_id),
                    roots: false,
                    search: None,
                })
                .await
                .with_context(|| {
                    format!("failed to list subtree children for session {parent_id}")
                })?;
            for child in children {
                if seen.insert(child.id) {
                    stack.push(child.id);
                    items.push(child);
                }
            }
        }

        Ok(items)
    }

    pub async fn list_session_timeline(
        &self,
        session_id: i64,
        limit: u64,
    ) -> Result<Vec<DomainEvent>> {
        let manager = self.session_manager()?;
        let mut all = manager
            .list_session_events(session_id)
            .await
            .context("failed to list session events")?;
        all.sort_by(|a, b| a.meta.seq_global.cmp(&b.meta.seq_global));
        if all.len() > limit as usize {
            all = all.split_off(all.len() - limit as usize);
        }
        Ok(all)
    }

    pub async fn list_all_messages(&self, session_id: i64) -> Result<Vec<MessageResource>> {
        let mut cursor = None;
        let mut messages = Vec::new();

        loop {
            let page = self
                .list_messages(session_id, cursor.clone(), 200)
                .await
                .context("failed to list full session message history")?;
            cursor = page.page.next_cursor.clone();
            messages.extend(page.items);
            if cursor.is_none() {
                break;
            }
        }

        messages.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(messages)
    }

    pub async fn get_session_state(&self, session_id: i64) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_query(
            &self.app_state,
            Query::GetSessionState(GetSessionParams { session_id }),
        )
        .await
        .map_err(api_error)?
        {
            QueryResult::SessionState(state) => Ok(state),
            other => Err(anyhow!("unexpected query result: {:?}", other)),
        }
        .context("failed to load session state")
    }

    pub async fn list_messages(
        &self,
        session_id: i64,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<PaginatedResponse<MessageResource>> {
        match dispatch::dispatch_query(
            &self.app_state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor,
                limit: Some(limit),
                parts: PartLoadMode::Full,
            }),
        )
        .await
        .map_err(api_error)?
        {
            QueryResult::Messages(page) => Ok(page),
            other => Err(anyhow!("unexpected query result: {:?}", other)),
        }
        .context("failed to list session messages")
    }

    pub async fn refresh_session(
        &self,
        session_id: i64,
        after_seq: Option<i64>,
        latest_message_limit: u64,
        force: bool,
    ) -> Result<SessionRefresh> {
        let manager = self.session_manager()?;
        let all_events = manager
            .list_session_events(session_id)
            .await
            .context("failed to fetch latest session event sequence")?;
        let latest_event_seq = all_events.iter().map(|event| event.meta.seq_global).max();
        let changed = force
            || match (after_seq, latest_event_seq) {
                (None, Some(_)) => true,
                (Some(after), Some(current)) => current > after,
                _ => false,
            };

        if !changed {
            return Ok(SessionRefresh {
                latest_event_seq,
                event_count: 0,
                execution: None,
                latest_messages: None,
            });
        }

        let event_count = match after_seq {
            Some(after) => all_events
                .iter()
                .filter(|event| event.meta.seq_global > after)
                .take(256)
                .count(),
            None => 0,
        };

        let execution = self.get_session_state(session_id).await?;
        let latest_messages = self
            .list_messages(session_id, None, latest_message_limit)
            .await
            .context("failed to refresh latest message window")?;

        Ok(SessionRefresh {
            latest_event_seq,
            event_count,
            execution: Some(execution),
            latest_messages: Some(latest_messages),
        })
    }

    /// Subscribe to live events for a session via the unified
    /// [`agena_event::EventBus`]. Replaces the legacy 250ms REST polling
    /// loop: callers receive a [`LiveEvent`] for every domain event the
    /// session emits, in real time.
    ///
    /// Returns `None` when the runtime has no session manager configured
    /// (e.g. a database-less smoke test).
    pub fn subscribe_session_events(
        &self,
        session_id: i64,
    ) -> Option<mpsc::UnboundedReceiver<LiveEvent>> {
        let manager = self.runtime.session_manager()?;
        let bus = manager.event_bus();
        let (tx, rx) = mpsc::unbounded_channel::<LiveEvent>();
        let mut subscription = bus.subscribe(EventFilter::new(Scope::Session { session_id }));
        tokio::spawn(async move {
            while let Some(item) = subscription.recv().await {
                let SubscriptionItem::Event(event) = item else {
                    // `Lagged(n)` => UI should ask for a forced refresh; we
                    // emit a stub LiveEvent with `triggers_refresh = true`
                    // so the existing refresh handler runs.
                    continue;
                };
                let triggers_refresh = matches!(
                    event.kind,
                    EventKind::AssistantMessageCompleted(_)
                        | EventKind::ToolCallCompleted(_)
                        | EventKind::TurnCompleted(_)
                        | EventKind::TurnAborted(_)
                        | EventKind::SystemNoticeAppended(_)
                        | EventKind::MessageRevised(_)
                        | EventKind::RunFailed(_)
                );
                let live = LiveEvent { triggers_refresh };
                if tx.send(live).is_err() {
                    break;
                }
            }
        });
        Some(rx)
    }

    pub async fn submit_parts_turn_with_options(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::SubmitTurn(SubmitTurnParams {
                session_id,
                options: request,
                parts,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to submit user turn")
    }

    pub fn prepare_attachment_from_path(&self, path: &Path) -> Result<AttachmentItem> {
        let resolved = self.resolve_workspace_path(path);

        if !resolved.exists() {
            return Err(anyhow!(
                "attachment path does not exist: {}",
                resolved.display()
            ));
        }
        if !resolved.is_file() {
            return Err(anyhow!(
                "attachment path is not a file: {}",
                resolved.display()
            ));
        }

        let bytes = fs::read(&resolved)
            .with_context(|| format!("failed to read attachment {}", resolved.display()))?;
        if bytes.is_empty() {
            return Err(anyhow!("attachment is empty: {}", resolved.display()));
        }
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(anyhow!(
                "attachments larger than {} bytes are not supported: {}",
                MAX_ATTACHMENT_BYTES,
                resolved.display()
            ));
        }

        let filename = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| resolved.display().to_string());
        let mime = detect_mime(&resolved, &bytes);
        let kind = AttachmentKind::detect(mime.as_str(), Some(filename.as_str()));
        let (width, height) = match kind {
            AttachmentKind::Image => detect_dimensions(&bytes),
            _ => (None, None),
        };

        Ok(AttachmentItem {
            kind,
            mime,
            source: AttachmentSource::Base64 {
                data: STANDARD.encode(&bytes),
            },
            filename: Some(filename),
            title: None,
            size_bytes: Some(bytes.len() as u64),
            sha256: None,
            width,
            height,
            duration_ms: None,
            page_count: None,
        })
    }

    pub fn resolve_workspace_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        }
    }

    pub fn memory_index_path(&self) -> Result<PathBuf> {
        let store = self.memory_store();
        store
            .ensure_exists()
            .context("failed to create memory directory")?;
        let path = store.dir().join("MEMORY.md");
        if !path.exists() {
            fs::write(&path, "")
                .with_context(|| format!("failed to create memory index {}", path.display()))?;
        }
        Ok(path)
    }

    pub fn memory_entry_path(&self, name: &str) -> Result<PathBuf> {
        self.memory_store()
            .get(name)
            .with_context(|| format!("failed to load memory `{name}`"))
            .map(|entry| entry.path)
    }

    pub fn forget_memory(&self, name: &str) -> Result<()> {
        self.memory_store()
            .forget(name)
            .with_context(|| format!("failed to forget memory `{name}`"))
    }

    pub fn search_workspace_files(&self, query: &str, limit: usize) -> Result<Vec<PathBuf>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let trimmed = query.trim();
        let query_lower = trimmed.to_lowercase();
        let index = self
            .file_index
            .get_or_init(|| build_file_index(&self.workspace_root));

        let mut matches = index
            .iter()
            .filter_map(|path| {
                let score = file_search_score(path, query_lower.as_str())?;
                Some((score, path.clone()))
            })
            .collect::<Vec<_>>();

        if let Some(path) = direct_path_candidate(&self.workspace_root, trimmed) {
            let already_present = matches.iter().any(|(_, existing)| existing == &path);
            if !already_present {
                matches.push(((0, 0, 0), path));
            }
        }

        matches.sort_by(|(score_a, path_a), (score_b, path_b)| {
            score_a
                .cmp(score_b)
                .then_with(|| {
                    path_a
                        .components()
                        .count()
                        .cmp(&path_b.components().count())
                })
                .then_with(|| path_a.as_os_str().len().cmp(&path_b.as_os_str().len()))
                .then_with(|| path_a.cmp(path_b))
        });
        matches.truncate(limit);
        Ok(matches.into_iter().map(|(_, path)| path).collect())
    }

    pub async fn continue_session_with_options(
        &self,
        session_id: i64,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::ContinueRun(ContinueRunParams {
                session_id,
                options: request,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to continue session")
    }

    pub async fn compact_session_with_options(
        &self,
        session_id: i64,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        let options = self
            .resolve_session_run_options(session_id, request)
            .await?;
        self.session_manager()?
            .compact_session(session_id, options)
            .await
            .context("failed to compact session")?;
        self.get_session_state(session_id).await
    }

    /// Best-effort cancel of the in-flight turn for `session_id`. Forwards
    /// to `SessionManager::cancel_active_turn`; the manager owns the
    /// `CancellationToken` for the spawned turn task. If no turn is
    /// active this is a no-op.
    pub async fn cancel_turn(&self, session_id: i64) -> Result<()> {
        self.session_manager()?
            .cancel_active_turn(session_id)
            .await
            .context("failed to cancel active turn")
    }

    /// Inject `parts` as a steer message into the in-flight turn. Returns
    /// `Err` when there is no active turn or the turn is in a phase that
    /// no longer accepts steers (the caller should re-queue).
    pub async fn steer_input(&self, session_id: i64, parts: Vec<PartContent>) -> Result<()> {
        self.session_manager()?
            .steer_input(session_id, parts)
            .await
            .context("failed to steer turn")
    }

    pub async fn reply_permission_with_options(
        &self,
        session_id: i64,
        request_id: String,
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::ReplyPermission(ReplyPermissionParams {
                session_id,
                options: request,
                reply: PermissionReply {
                    request_id,
                    kind,
                    reason: None,
                    scope,
                },
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to reply to permission request")
    }

    pub async fn reply_user_input_with_options(
        &self,
        session_id: i64,
        reply: UserInputReply,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::ReplyUserInput(ReplyUserInputParams {
                session_id,
                options: request,
                reply,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to submit user input reply")
    }

    pub async fn rewind_session_to_message(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<SessionExecutionResource> {
        let expected_version = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?
            .version;
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::RewindSession(RewindSessionParams {
                session_id,
                message_id,
                expected_version: Some(expected_version),
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Execution(state) => Ok(state),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to rewind session to message")
    }

    pub fn plugin_statusline_segments(&self) -> Vec<agena::plugin::HostStatuslineSegment> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .statusline_segments()
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn workspace_name(&self) -> String {
        self.workspace_root
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| self.workspace_root.display().to_string())
    }

    pub fn plugin_theme_palettes(&self) -> Vec<agena::plugin::HostThemePalette> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .theme_palettes()
    }

    pub fn plugin_statuses(&self) -> Vec<agena::plugin::status::PluginStatus> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_statuses()
    }

    pub fn plugin_inspect(&self, plugin_id: &str) -> Option<agena::plugin::PluginInspect> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_inspect(plugin_id)
    }

    pub fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena::plugin::PluginLogEntry> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_logs(plugin_id, after_seq, limit)
    }

    pub fn runtime_inspector_rows(&self) -> Vec<InspectorRow> {
        let snapshot = self.runtime.current_snapshot();
        let resolution = snapshot.config_resolution();
        let mut rows = vec![
            InspectorRow {
                label: "generation".to_string(),
                detail: snapshot.generation().to_string(),
            },
            InspectorRow {
                label: "workspace_root".to_string(),
                detail: self.workspace_root.display().to_string(),
            },
            InspectorRow {
                label: "config_path".to_string(),
                detail: resolution.meta.config_path.display().to_string(),
            },
            InspectorRow {
                label: "providers".to_string(),
                detail: snapshot.provider_registry().provider_ids().join(", "),
            },
            InspectorRow {
                label: "plugins".to_string(),
                detail: snapshot.plugin_manager().plugins().len().to_string(),
            },
            InspectorRow {
                label: "watch_paths".to_string(),
                detail: snapshot
                    .watch_paths()
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" | "),
            },
        ];
        if let Some(manager) = snapshot.session_manager() {
            let stats = manager.cache_stats();
            rows.push(InspectorRow {
                label: "session_cache".to_string(),
                detail: format!(
                    "entries={} hits={} misses={} evictions={}",
                    stats.entry_count, stats.hits, stats.misses, stats.evictions
                ),
            });
        }
        rows
    }

    pub fn mcp_inspector_rows(&self) -> Vec<InspectorRow> {
        let snapshot = self.runtime.current_snapshot();
        let mut rows = snapshot
            .config_resolution()
            .config
            .mcp
            .servers
            .iter()
            .map(|(name, config)| InspectorRow {
                label: name.clone(),
                detail: match config {
                    agena::config::McpServerConfig::Stdio { command, args, .. } => {
                        format!("stdio {} {}", command, args.join(" "))
                    }
                    agena::config::McpServerConfig::Http { url, mode, .. } => {
                        format!("http {} {:?}", url, mode)
                    }
                },
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        rows
    }

    pub fn lsp_inspector_rows(&self) -> Vec<InspectorRow> {
        let snapshot = self.runtime.current_snapshot();
        let mut rows = snapshot
            .config_resolution()
            .config
            .lsp
            .servers
            .iter()
            .map(|(name, config)| InspectorRow {
                label: name.clone(),
                detail: format!(
                    "{} | ext={} | roots={}",
                    config.command,
                    if config.file_extensions.is_empty() {
                        "all".to_string()
                    } else {
                        config.file_extensions.join(",")
                    },
                    if config.root_markers.is_empty() {
                        "workspace".to_string()
                    } else {
                        config.root_markers.join(",")
                    }
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        rows
    }

    pub fn skills_inspector_rows(&self) -> Vec<InspectorRow> {
        let mut rows = self
            .runtime
            .current_snapshot()
            .plugin_manager()
            .entry_entries()
            .into_iter()
            .filter(|entry| entry.plugin_name == "agena.skills")
            .map(|entry| InspectorRow {
                label: entry.exposed_name,
                detail: format!(
                    "{} | {}",
                    entry.plugin_name,
                    entry.decl.description.unwrap_or_default()
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        rows
    }

    pub fn runtime_entry_rows(&self) -> Vec<InspectorRow> {
        let mut rows = self
            .runtime
            .current_snapshot()
            .plugin_manager()
            .entry_entries()
            .into_iter()
            .map(|entry| InspectorRow {
                label: entry.exposed_name,
                detail: format!(
                    "{} | {}",
                    entry.plugin_name,
                    entry.decl.description.unwrap_or_default()
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        rows
    }

    pub async fn session_cost_inspector_rows(&self, session_id: i64) -> Result<Vec<InspectorRow>> {
        let session = self.session_manager()?.get_session(session_id).await?;
        let summary = agena::session::cost::summarize(&session.messages);
        let mut rows = vec![
            InspectorRow {
                label: "summary".to_string(),
                detail: summary.one_line(),
            },
            InspectorRow {
                label: "turns".to_string(),
                detail: summary.turns.to_string(),
            },
            InspectorRow {
                label: "input_tokens".to_string(),
                detail: summary.input_tokens.to_string(),
            },
            InspectorRow {
                label: "output_tokens".to_string(),
                detail: summary.output_tokens.to_string(),
            },
            InspectorRow {
                label: "reasoning_tokens".to_string(),
                detail: summary.reasoning_tokens.to_string(),
            },
            InspectorRow {
                label: "cache_write_tokens".to_string(),
                detail: summary.cache_write_tokens.to_string(),
            },
            InspectorRow {
                label: "cache_read_tokens".to_string(),
                detail: summary.cache_read_tokens.to_string(),
            },
            InspectorRow {
                label: "total_cost_usd".to_string(),
                detail: format!("{:.4}", summary.total_cost_usd),
            },
        ];
        rows.extend(summary.by_model.into_iter().map(|entry| InspectorRow {
            label: format!("{}/{}", entry.provider_id, entry.model_id),
            detail: format!(
                "turns={} in={} out={} reasoning={} cost=${:.4}",
                entry.turns,
                entry.input_tokens,
                entry.output_tokens,
                entry.reasoning_tokens,
                entry.total_cost_usd
            ),
        }));
        Ok(rows)
    }

    pub async fn list_permission_rules(&self) -> Result<Vec<PermissionRuleResource>> {
        match dispatch::dispatch_query(
            &self.app_state,
            Query::ListPermissionRules(ListPermissionRulesParams {
                cursor: None,
                limit: Some(200),
                search: None,
            }),
        )
        .await
        .map_err(api_error)?
        {
            QueryResult::PermissionRules(page) => {
                let mut rules = page.items;
                rules.sort_by(|left, right| left.action_key.cmp(&right.action_key));
                Ok(rules)
            }
            other => Err(anyhow!("unexpected query result: {:?}", other)),
        }
        .context("failed to list permission rules")
    }

    pub async fn create_permission_rule(
        &self,
        params: UpsertPermissionRuleParams,
    ) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(&self.app_state, ApiCommand::UpsertPermissionRule(params))
            .await
            .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to create permission rule")
    }

    pub async fn replace_permission_rule(
        &self,
        rule_id: i64,
        params: UpsertPermissionRuleParams,
    ) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::ReplacePermissionRule(ReplacePermissionRuleParams {
                rule_id,
                rule: params,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to replace permission rule")
    }

    pub async fn revoke_permission_rule(&self, rule_id: i64) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::RevokePermissionRule(agena_api::commands::RevokePermissionRuleParams {
                rule_id,
                reason: None,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to revoke permission rule")
    }

    pub fn config_inspector_rows(&self) -> Vec<InspectorRow> {
        let snapshot = self.runtime.current_snapshot();
        let resolution = snapshot.config_resolution();
        let mut rows = vec![
            InspectorRow {
                label: "config_path".to_string(),
                detail: resolution.meta.config_path.display().to_string(),
            },
            InspectorRow {
                label: "config_found".to_string(),
                detail: resolution.meta.config_found.to_string(),
            },
            InspectorRow {
                label: "provider_count".to_string(),
                detail: resolution.config.providers.len().to_string(),
            },
            InspectorRow {
                label: "plugin_entries".to_string(),
                detail: resolution
                    .config
                    .plugins
                    .list
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | "),
            },
            InspectorRow {
                label: "mcp_servers".to_string(),
                detail: resolution.config.mcp.servers.len().to_string(),
            },
            InspectorRow {
                label: "lsp_servers".to_string(),
                detail: resolution.config.lsp.servers.len().to_string(),
            },
        ];
        rows.retain(|row| !row.detail.is_empty());
        rows
    }

    pub fn worktree_inspector_rows(&self) -> Vec<InspectorRow> {
        let Some(manager) = self.runtime.session_manager() else {
            return vec![InspectorRow {
                label: "session_runtime".to_string(),
                detail: "unavailable".to_string(),
            }];
        };
        let executor = manager.tool_executor();
        let Some(registry) = executor.worktree_registry() else {
            return vec![InspectorRow {
                label: "worktree_registry".to_string(),
                detail: "unavailable".to_string(),
            }];
        };
        let active = tool::worktree_list_active(registry);
        let managed = tool::worktree_list_managed(&self.workspace_root, registry);
        let mut rows = vec![
            InspectorRow {
                label: "active_sessions".to_string(),
                detail: active.len().to_string(),
            },
            InspectorRow {
                label: "managed_dirs".to_string(),
                detail: managed.len().to_string(),
            },
        ];
        rows.extend(active.into_iter().map(|entry| InspectorRow {
            label: format!("session #{}", entry.session_id),
            detail: format!(
                "{} | branch={} | created_here={}",
                entry.path.display(),
                entry.branch,
                entry.created_here
            ),
        }));
        rows.extend(managed.into_iter().map(|entry| {
            let session_id = entry
                .session_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_string());
            let branch = entry
                .branch
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let stale = entry.is_stale();
            InspectorRow {
                label: entry.path.display().to_string(),
                detail: format!(
                    "session={} | branch={} | git_registered={} | stale={}",
                    session_id, branch, entry.registered_with_git, stale
                ),
            }
        }));
        rows
    }

    pub async fn git_inspector_rows(&self) -> Result<Vec<InspectorRow>> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        Ok(git_status_inspector_rows(status))
    }

    pub fn enter_worktree(
        &self,
        session_id: i64,
        name: Option<String>,
        path: Option<String>,
    ) -> Result<WorktreeCommandOutput> {
        let manager = self.session_manager()?;
        let output = manager
            .tool_executor()
            .execute_tool_payload_for_host(
                "enter_worktree",
                serde_json::to_value(EnterWorktreeToolInput { name, path })?,
                Some(session_id),
                None,
                None,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
        parse_worktree_payload(output.payload)
    }

    pub fn exit_worktree(
        &self,
        session_id: i64,
        action: String,
        discard_changes: bool,
    ) -> Result<WorktreeCommandOutput> {
        let manager = self.session_manager()?;
        let output = manager
            .tool_executor()
            .execute_tool_payload_for_host(
                "exit_worktree",
                serde_json::to_value(ExitWorktreeToolInput {
                    action,
                    discard_changes,
                })?,
                Some(session_id),
                None,
                None,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
        parse_worktree_payload(output.payload)
    }

    pub fn runtime_entry_exists(&self, name: &str) -> bool {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .lookup_entry(name)
            .is_some()
    }

    pub fn runtime_entry_prompt(&self, session_id: i64, name: &str, args: &str) -> Result<String> {
        let manager = self.session_manager()?;
        let invocation = ToolInvocation::new(
            name.to_string(),
            serde_json::from_value::<agena::message::StructuredObject>(json!({
                "args": if args.trim().is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(args.trim().to_string())
                }
            }))
            .map_err(|error| anyhow!(error))?,
        );
        let execution = manager
            .tool_executor()
            .execute_invocation_detailed(&invocation, session_id, -1)
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(execution.view.output_text)
    }

    pub async fn create_commit(&self, message: String) -> Result<(String, String)> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        if !status.git_available {
            return Err(anyhow!("git is not available in PATH"));
        }
        if !status.repo {
            return Err(anyhow!(
                "not a git repository: {}",
                self.workspace_root.display()
            ));
        }
        if status.staged_files == 0 {
            return Err(anyhow!("no staged changes to commit"));
        }

        let output = Command::new("git")
            .args(["commit", "-m", message.as_str()])
            .current_dir(&self.workspace_root)
            .output()
            .context("failed to execute git commit")?;
        if !output.status.success() {
            return Err(anyhow!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let commit = git_command_output(&self.workspace_root, ["rev-parse", "HEAD"])?;
        let summary = git_command_output(&self.workspace_root, ["log", "-1", "--pretty=%s"])?;
        Ok((commit, summary))
    }

    pub async fn create_pr(
        &self,
        title: String,
        body: Option<String>,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<String> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        if !status.git_available {
            return Err(anyhow!("git is not available in PATH"));
        }
        if !status.gh_available {
            return Err(anyhow!("gh is not available in PATH"));
        }
        if !status.repo {
            return Err(anyhow!(
                "not a git repository: {}",
                self.workspace_root.display()
            ));
        }

        let branch = head
            .clone()
            .or(status.branch.clone())
            .ok_or_else(|| anyhow!("could not determine current branch"))?;

        let mut command = Command::new("gh");
        command.arg("pr").arg("create").arg("--title").arg(title);
        command.arg("--body").arg(body.unwrap_or_default());
        if let Some(base) = base {
            command.arg("--base").arg(base);
        }
        command.arg("--head").arg(branch);
        command.current_dir(&self.workspace_root);

        let output = command.output().context("failed to execute gh pr create")?;
        if !output.status.success() {
            return Err(anyhow!(
                "gh pr create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    async fn resolve_workspace_resource(
        &self,
        create_if_missing: bool,
    ) -> Result<WorkspaceResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::ResolveWorkspace(agena_api::commands::ResolveWorkspaceParams {
                path: self.workspace_root.to_string_lossy().to_string(),
                create_if_missing,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Workspace(workspace) => Ok(workspace),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to resolve workspace")
    }

    async fn git_status(&self) -> Result<GitStatusResource> {
        let workspace_root = self.runtime.workspace_root().to_path_buf();
        let git_available = command_available("git");
        let gh_available = command_available("gh");

        let Some(manager) = self.runtime.session_manager() else {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo: false,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                worktree_active_sessions: 0,
                worktree_managed_dirs: 0,
            });
        };

        let executor = manager.tool_executor();
        let (worktree_active_sessions, worktree_managed_dirs) = match executor.worktree_registry() {
            Some(registry) => (
                tool::worktree_list_active(registry).len() as u64,
                tool::worktree_list_managed(&workspace_root, registry).len() as u64,
            ),
            None => (0, 0),
        };

        if !git_available {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo: false,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                worktree_active_sessions,
                worktree_managed_dirs,
            });
        }

        let repo = git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]);
        if !repo {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                worktree_active_sessions,
                worktree_managed_dirs,
            });
        }

        let branch = git_command_output(&workspace_root, ["branch", "--show-current"])?;
        let upstream = git_command_output(
            &workspace_root,
            [
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )
        .ok()
        .and_then(|value| non_empty(Some(value.as_str())).map(ToOwned::to_owned));
        let ahead_behind = upstream.as_ref().and_then(|_| {
            git_command_output(
                &workspace_root,
                ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            )
            .ok()
        });
        let (ahead, behind) = parse_ahead_behind(ahead_behind.as_deref());
        let status = git_command_output(&workspace_root, ["status", "--porcelain"])?;
        let (staged_files, unstaged_files, untracked_files, changed_files) =
            summarize_git_status(status.as_str());

        Ok(GitStatusResource {
            workspace_root: workspace_root.display().to_string(),
            git_available,
            repo,
            gh_available,
            branch: non_empty(Some(branch.as_str())).map(ToOwned::to_owned),
            upstream,
            ahead,
            behind,
            staged_files,
            unstaged_files,
            untracked_files,
            changed_files,
            clean: changed_files == 0,
            worktree_active_sessions,
            worktree_managed_dirs,
        })
    }

    pub fn resolve_model_target(&self, target: &str, model: Option<&str>) -> Result<ModelRef> {
        self.runtime
            .current_snapshot()
            .resolve_model_target(target, model)
            .context("failed to resolve model target")
    }

    async fn current_workspace_id(&self) -> Result<i64> {
        Ok(self
            .resolve_workspace_resource(true)
            .await
            .context("failed to resolve current workspace")?
            .id)
    }

    async fn list_sessions_query(&self, query: ListSessionsParams) -> Result<Vec<SessionResource>> {
        let mut cursor = query.cursor.clone();
        let limit = query.limit.unwrap_or(200);
        let mut items = Vec::new();

        loop {
            let page = match dispatch::dispatch_query(
                &self.app_state,
                Query::ListSessions(ListSessionsParams {
                    cursor: cursor.clone(),
                    limit: Some(limit),
                    workspace_id: query.workspace_id,
                    parent_id: query.parent_id,
                    roots: query.roots,
                    search: query.search.clone(),
                }),
            )
            .await
            .map_err(api_error)?
            {
                QueryResult::Sessions(page) => page,
                other => return Err(anyhow!("unexpected query result: {:?}", other)),
            };
            cursor = page.page.next_cursor.clone();
            items.extend(page.items);
            if !page.page.has_more || cursor.is_none() {
                break;
            }
        }

        Ok(items)
    }

    async fn resolve_session_root(&self, session_id: i64) -> Result<SessionResource> {
        let mut current = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        while let Some(parent_id) = current.parent_id {
            current = self
                .get_session(parent_id)
                .await?
                .ok_or_else(|| anyhow!("session not found: {parent_id}"))?;
        }
        Ok(current)
    }

    async fn resolve_session_run_options(
        &self,
        session_id: i64,
        request: RunOptions,
    ) -> Result<agena::session::SessionRunOptions> {
        let RunOptions {
            model,
            variant,
            agent_profile,
            system,
            temperature,
            max_output_tokens,
            max_turn_loops,
        } = request;

        let mut options = self
            .session_manager()?
            .resolve_scheduled_run_options(session_id)
            .await
            .context("failed to resolve session run options")?;

        if let Some(model) = model {
            options.model = model;
        }
        if let Some(variant) = variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let snapshot = self.runtime.current_snapshot();
            let provider_registry = snapshot.provider_registry();
            let variants = provider_registry
                .model_variants(&options.model)
                .context("failed to resolve selected model variants")?;
            let definition = variants
                .get(variant)
                .ok_or_else(|| anyhow!("model {} has no variant {variant}", options.model))?;
            let adapter_id = options
                .model
                .adapter_id
                .as_ref()
                .map(|value| value.to_string())
                .or_else(|| {
                    provider_registry
                        .get(options.model.provider_id.as_str())
                        .and_then(|provider| {
                            provider.default_adapter().map(|value| value.to_string())
                        })
                });
            let mut request_override = definition.request_override.clone();
            if let Some(adapter_id) = adapter_id.as_deref()
                && let Some(adapter_override) = definition.adapter_overrides.get(adapter_id)
            {
                request_override = request_override.merged_with(adapter_override);
            }
            options.variant = Some(variant.to_string());
            options.thinking = definition.thinking.clone();
            options.request_override = request_override;
        }

        if let Some(system) = system {
            options.system = Some(system);
        }
        if let Some(temperature) = temperature {
            options.temperature = Some(temperature);
        }
        if let Some(max_output_tokens) = max_output_tokens {
            options.max_output_tokens = Some(max_output_tokens);
        }
        if let Some(agent_profile) = agent_profile {
            options.agent_profile = Some(agent_profile);
        }
        if let Some(max_turn_loops) = max_turn_loops {
            options.max_turn_loops = Some(max_turn_loops);
        }

        Ok(options)
    }

    fn session_manager(&self) -> Result<Arc<agena::session::SessionManager>> {
        self.runtime
            .session_manager()
            .ok_or_else(|| anyhow!("session runtime is not available"))
    }

    fn memory_store(&self) -> MemoryStore {
        MemoryStore::for_workspace(&self.workspace_root)
    }
}

fn build_file_index(workspace_root: &Path) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(workspace_root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .follow_links(false)
        .parents(true)
        .require_git(false);

    let mut files = builder
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(workspace_root)
                .ok()
                .map(Path::to_path_buf)
        })
        .collect::<Vec<_>>();

    files.sort();
    files
}

fn file_search_score(path: &Path, query_lower: &str) -> Option<(u8, usize, usize)> {
    if query_lower.is_empty() {
        return Some((4, path.components().count(), path.as_os_str().len()));
    }

    let path_text = path.to_string_lossy();
    let path_lower = path_text.to_lowercase();
    let filename_lower = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if filename_lower == query_lower {
        return Some((0, filename_lower.len(), path_lower.len()));
    }
    if filename_lower.starts_with(query_lower) {
        return Some((1, filename_lower.len(), path_lower.len()));
    }
    if let Some(index) = filename_lower.find(query_lower) {
        return Some((2, index, path_lower.len()));
    }

    path_lower
        .find(query_lower)
        .map(|index| (3, index, path_lower.len()))
}

fn direct_path_candidate(workspace_root: &Path, query: &str) -> Option<PathBuf> {
    if query.is_empty() {
        return None;
    }

    let typed = Path::new(query);
    let resolved = if typed.is_absolute() {
        typed.to_path_buf()
    } else {
        workspace_root.join(typed)
    };
    if !resolved.is_file() {
        return None;
    }

    resolved
        .strip_prefix(workspace_root)
        .map(Path::to_path_buf)
        .ok()
        .or(Some(resolved))
}

fn api_error(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow!(error.to_string())
}

fn optional_non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn optional_string(value: &str) -> Option<String> {
    optional_non_empty(value).map(str::to_owned)
}

fn required_trimmed<'a>(value: &'a str, field: &str) -> Result<&'a str> {
    optional_non_empty(value).ok_or_else(|| anyhow!("{field} is required"))
}

fn local_model_catalog_summary(
    catalog: &agena::model_catalog::ModelCatalogResponse,
) -> LocalModelCatalogResponse {
    let official_entry_count = catalog
        .entries
        .iter()
        .filter(|entry| !entry.has_local_override)
        .count();
    let custom_entry_count = catalog
        .entries
        .iter()
        .filter(|entry| entry.has_local_override)
        .count();
    LocalModelCatalogResponse {
        last_refresh_at: catalog.last_refresh_at,
        last_successful_source: catalog.last_successful_source,
        last_error: catalog.last_error.clone(),
        entry_count: catalog.entries.len(),
        official_entry_count,
        custom_entry_count,
    }
}

fn local_model_catalog_entry_resources(
    catalog: &agena::model_catalog::ModelCatalogResponse,
) -> Vec<ModelCatalogEntryResource> {
    catalog
        .entries
        .iter()
        .cloned()
        .map(|entry| ModelCatalogEntryResource::from_record(entry, catalog.last_successful_source))
        .collect()
}

fn local_model_catalog_entry_search_text(entry: &ModelCatalogEntryResource) -> String {
    let variant_text = entry
        .variants
        .iter()
        .flat_map(|(name, variant)| {
            [
                name.clone(),
                variant.display_name.clone().unwrap_or_default(),
                variant.description.clone().unwrap_or_default(),
                variant
                    .thinking
                    .as_ref()
                    .and_then(|value| serde_json::to_string(value).ok())
                    .unwrap_or_default(),
                serde_json::to_string(&variant.request_override).unwrap_or_default(),
                serde_json::to_string(&variant.adapter_overrides).unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n");

    [
        entry.model_id.clone(),
        entry.display_name.clone().unwrap_or_default(),
        entry.origin.clone().unwrap_or_default(),
        entry.description.clone().unwrap_or_default(),
        match entry.kind {
            ModelCatalogEntryKind::Official => "official".to_owned(),
            ModelCatalogEntryKind::Custom => "custom".to_owned(),
        },
        match entry.source {
            ModelCatalogSourceKind::Generated => "generated".to_owned(),
            ModelCatalogSourceKind::Cache => "cache".to_owned(),
            ModelCatalogSourceKind::Custom => "custom".to_owned(),
        },
        entry.source_label.clone().unwrap_or_default(),
        entry
            .lifecycle
            .map(|value| match value {
                agena::model::ModelLifecycle::Active => "active",
                agena::model::ModelLifecycle::Preview => "preview",
                agena::model::ModelLifecycle::Beta => "beta",
                agena::model::ModelLifecycle::Alpha => "alpha",
                agena::model::ModelLifecycle::Experimental => "experimental",
                agena::model::ModelLifecycle::Deprecated => "deprecated",
            })
            .unwrap_or_default()
            .to_owned(),
        variant_text,
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase()
}

fn preferred_catalog_entry_for_model_id<'a>(
    entries: &'a [ModelCatalogEntryResource],
    model_id: &str,
) -> Option<&'a ModelCatalogEntryResource> {
    preferred_catalog_entry_for_lookup_ids(entries, &[model_id.to_owned()])
}

fn preferred_catalog_entry_for_lookup_ids<'a>(
    entries: &'a [ModelCatalogEntryResource],
    model_ids: &[String],
) -> Option<&'a ModelCatalogEntryResource> {
    let lookup_ids = model_ids
        .iter()
        .map(|model_id| model_id.trim())
        .filter(|model_id| !model_id.is_empty())
        .collect::<Vec<_>>();
    entries
        .iter()
        .filter(|entry| {
            lookup_ids
                .iter()
                .any(|model_id| entry.model_id == *model_id)
        })
        .min_by_key(|entry| match entry.kind {
            ModelCatalogEntryKind::Custom => 0,
            ModelCatalogEntryKind::Official => 1,
        })
}

fn preferred_catalog_entry_for_provider_model<'a>(
    entries: &'a [ModelCatalogEntryResource],
    provider_model: &ProviderModel,
) -> Option<&'a ModelCatalogEntryResource> {
    preferred_catalog_entry_for_lookup_ids(
        entries,
        &[
            provider_model.id.to_string(),
            catalog_lookup_id_for_provider_model(provider_model),
        ],
    )
}

fn catalog_lookup_id_for_model_id(model_id: &str) -> String {
    agena::model_catalog::canonical_model_catalog_id(model_id)
}

fn catalog_lookup_id_for_provider_model(provider_model: &ProviderModel) -> String {
    provider_model
        .catalog_model_id
        .as_ref()
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| catalog_lookup_id_for_model_id(provider_model.id.as_str()))
}

fn provider_model_json_for_model_id(
    catalog_entries: &[ModelCatalogEntryResource],
    model_id: &str,
    provider_model: Option<&ProviderModel>,
) -> JsonValue {
    preferred_catalog_entry_for_model_id(catalog_entries, model_id)
        .or_else(|| {
            let lookup_id = catalog_lookup_id_for_model_id(model_id);
            (lookup_id != model_id)
                .then(|| preferred_catalog_entry_for_model_id(catalog_entries, lookup_id.as_str()))
                .flatten()
        })
        .map(catalog_entry_to_provider_model_value)
        .or_else(|| {
            provider_model.and_then(|provider_model| {
                preferred_catalog_entry_for_provider_model(catalog_entries, provider_model)
                    .map(catalog_entry_to_provider_model_value)
                    .or_else(|| Some(provider_model_to_provider_model_value(provider_model)))
            })
        })
        .unwrap_or_else(|| JsonValue::Object(JsonMap::new()))
}

fn catalog_entry_to_provider_model_value(entry: &ModelCatalogEntryResource) -> JsonValue {
    let mut value = JsonMap::new();
    if let Some(lifecycle) = entry.lifecycle {
        value.insert("lifecycle".to_owned(), json!(lifecycle));
    }
    if let Some(context_window_tokens) = entry.context_window_tokens {
        value.insert(
            "context_window_tokens".to_owned(),
            JsonValue::Number(context_window_tokens.into()),
        );
    }
    if let Some(max_output_tokens) = entry.max_output_tokens {
        value.insert(
            "max_output_tokens".to_owned(),
            JsonValue::Number(max_output_tokens.into()),
        );
    }
    if let Some(display_name) = non_empty(entry.display_name.as_deref()) {
        value.insert(
            "display_name".to_owned(),
            JsonValue::String(display_name.to_owned()),
        );
    }
    if let Some(description) = non_empty(entry.description.as_deref()) {
        value.insert(
            "description".to_owned(),
            JsonValue::String(description.to_owned()),
        );
    }
    if !entry.variants.is_empty() {
        value.insert("variants".to_owned(), json!(entry.variants));
    }
    if let Ok(JsonValue::Object(capabilities)) = serde_json::to_value(&entry.capabilities) {
        for (key, part) in capabilities {
            let is_empty_object = matches!(&part, JsonValue::Object(object) if object.is_empty());
            if !part.is_null() && !is_empty_object {
                value.insert(key, part);
            }
        }
    }
    JsonValue::Object(value)
}

fn provider_model_to_provider_model_value(model: &ProviderModel) -> JsonValue {
    let mut value = JsonMap::new();
    if let Some(display_name) = non_empty(model.display_name.as_deref()) {
        value.insert(
            "display_name".to_owned(),
            JsonValue::String(display_name.to_owned()),
        );
    }
    if let Some(lifecycle) = model.metadata.lifecycle {
        value.insert("lifecycle".to_owned(), json!(lifecycle));
    }
    if let Some(context_window_tokens) = model.metadata.limits.context_window_tokens {
        value.insert(
            "context_window_tokens".to_owned(),
            JsonValue::Number(context_window_tokens.into()),
        );
    }
    if let Some(max_output_tokens) = model.metadata.limits.max_output_tokens {
        value.insert(
            "max_output_tokens".to_owned(),
            JsonValue::Number(max_output_tokens.into()),
        );
    }
    if let Some(description) = non_empty(model.metadata.description.as_deref()) {
        value.insert(
            "description".to_owned(),
            JsonValue::String(description.to_owned()),
        );
    }
    if !model.variants.is_empty() {
        let variants = model
            .variants
            .iter()
            .map(|(name, variant)| {
                (
                    name.clone(),
                    json!({
                        "display_name": variant.display_name,
                        "description": variant.description,
                        "thinking": variant.thinking,
                        "request_override": variant.request_override,
                        "adapter_overrides": variant.adapter_overrides,
                    }),
                )
            })
            .collect::<JsonMap<String, JsonValue>>();
        value.insert("variants".to_owned(), JsonValue::Object(variants));
    }

    let mut supported_features = Vec::new();
    if model.capabilities.tool_calling.is_supported() {
        supported_features.push("tool_calling");
    }
    if model.capabilities.streaming.is_supported() {
        supported_features.push("streaming");
    }
    if model.capabilities.reasoning.is_supported() {
        supported_features.push("reasoning");
    }
    if model.capabilities.structured_output.is_supported() {
        supported_features.push("structured_output");
    }
    if model.capabilities.temperature_supported.is_supported() {
        supported_features.push("temperature");
    }
    if !supported_features.is_empty() {
        value.insert(
            "features".to_owned(),
            json!({ "supported": supported_features }),
        );
    }
    JsonValue::Object(value)
}

fn ensure_provider_model_entry(
    adapter_value: &mut JsonValue,
    model_id: &str,
    model_value: JsonValue,
) -> Result<()> {
    let adapter = adapter_value
        .as_object_mut()
        .ok_or_else(|| anyhow!("adapter patch must be an object"))?;
    adapter.insert("enabled".to_owned(), JsonValue::Bool(true));
    if !adapter.contains_key("models") {
        adapter.insert("models".to_owned(), JsonValue::Object(JsonMap::new()));
    }
    let models = adapter
        .get_mut("models")
        .and_then(JsonValue::as_object_mut)
        .ok_or_else(|| anyhow!("adapter models patch must be an object"))?;
    models.insert(model_id.to_owned(), model_value);
    Ok(())
}

fn build_provider_patch_value(
    draft: &ProviderConfigDraft,
    default_adapter: &str,
    default_model: &str,
    adapters: JsonValue,
    include_defaults: bool,
) -> Result<JsonValue> {
    let mut value = JsonMap::new();
    value.insert("enabled".to_owned(), JsonValue::Bool(true));
    if include_defaults {
        value.insert(
            "default_adapter".to_owned(),
            JsonValue::String(default_adapter.to_owned()),
        );
        value.insert(
            "default_model".to_owned(),
            JsonValue::String(default_model.to_owned()),
        );
    }
    if draft.source_provider_id.is_none() || draft.auth_kind.is_api() {
        let base_url = required_trimmed(draft.base_url.as_str(), "base_url")?;
        let mut auth = JsonMap::new();
        auth.insert("mode".to_owned(), JsonValue::String("api".to_owned()));
        auth.insert(
            "base_url".to_owned(),
            JsonValue::String(base_url.to_owned()),
        );
        if let Some(api_key_env) = non_empty(Some(draft.api_key_env.as_str())) {
            auth.insert(
                "api_key_env".to_owned(),
                JsonValue::String(api_key_env.to_owned()),
            );
        }
        if let Some(api_key) = non_empty(Some(draft.api_key.as_str())) {
            auth.insert("api_key".to_owned(), JsonValue::String(api_key.to_owned()));
        }
        value.insert("auth".to_owned(), JsonValue::Object(auth));
    }
    value.insert("adapters".to_owned(), adapters);
    Ok(JsonValue::Object(value))
}

fn draft_discovery_adapters(
    adapter_ids: &[String],
) -> Result<std::collections::BTreeMap<String, ResolvedProviderAdapterConfig>> {
    let requested = if adapter_ids.is_empty() {
        vec![
            "openai".to_owned(),
            "anthropic".to_owned(),
            "gemini".to_owned(),
        ]
    } else {
        adapter_ids.to_vec()
    };
    let mut adapters = std::collections::BTreeMap::new();
    for adapter_id in requested {
        let trimmed = adapter_id.trim();
        if trimmed.is_empty() {
            continue;
        }
        let config = match trimmed {
            "openai" => ResolvedProviderAdapterConfig {
                enabled: true,
                definition: ProviderAdapterDefinition::OpenAi(HttpProviderAdapterConfig {
                    extra_headers: std::collections::BTreeMap::new(),
                    options: OpenAiProviderOptions {
                        backend: OpenAiBackendConfig::Api,
                        api_mode: OpenAiApiModeConfig::Auto,
                        api_mode_explicit: false,
                        stream_mode: StreamTransportMode::Sse,
                        realtime_ws_url: None,
                        models_url: None,
                        auth_header: "authorization".to_owned(),
                        auth_scheme: Some("Bearer".to_owned()),
                        capability_family: None,
                    },
                }),
            },
            "anthropic" => ResolvedProviderAdapterConfig {
                enabled: true,
                definition: ProviderAdapterDefinition::Anthropic(HttpProviderAdapterConfig {
                    extra_headers: std::collections::BTreeMap::new(),
                    options: AnthropicProviderOptions {
                        models_url: None,
                        messages_url: None,
                        auth_header: "x-api-key".to_owned(),
                        auth_scheme: None,
                        extra_beta_header: None,
                        eager_input_streaming: None,
                    },
                }),
            },
            "gemini" => ResolvedProviderAdapterConfig {
                enabled: true,
                definition: ProviderAdapterDefinition::Gemini(HttpProviderAdapterConfig {
                    extra_headers: std::collections::BTreeMap::new(),
                    options: SimpleHttpProviderOptions {
                        auth_header: None,
                        auth_scheme: None,
                    },
                }),
            },
            _ => {
                return Err(anyhow!(
                    "draft adapter discovery does not support `{trimmed}`"
                ));
            }
        };
        adapters.insert(trimmed.to_owned(), config);
    }
    Ok(adapters)
}

fn saved_discovery_adapters(
    provider_id: &str,
    resolved: &config::ResolvedProviderConfig,
    auth: &ProviderAuthConfig,
    adapter_ids: &[String],
) -> Result<std::collections::BTreeMap<String, ResolvedProviderAdapterConfig>> {
    if adapter_ids.is_empty() {
        let mut adapters = resolved.adapters.clone();
        if matches!(
            auth,
            ProviderAuthConfig::Api(_)
                | ProviderAuthConfig::GoogleAdc(_)
                | ProviderAuthConfig::SapAiCore(_)
        ) {
            for (adapter_id, adapter) in draft_discovery_adapters(&[])? {
                adapters.entry(adapter_id).or_insert(adapter);
            }
        }
        return Ok(adapters);
    }

    let mut adapters = std::collections::BTreeMap::new();
    for adapter_id in adapter_ids {
        let trimmed = adapter_id.trim();
        if trimmed.is_empty() {
            continue;
        }
        let adapter = match resolved.adapters.get(trimmed).cloned() {
            Some(adapter) => adapter,
            None => {
                let mut discovered = draft_discovery_adapters(&[trimmed.to_owned()])?;
                discovered.remove(trimmed).ok_or_else(|| {
                    anyhow!("provider {provider_id} does not define adapter `{trimmed}`")
                })?
            }
        };
        adapters.insert(trimmed.to_owned(), adapter);
    }
    Ok(adapters)
}

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn git_success<const N: usize>(workspace_root: &Path, args: [&str; N]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn parse_ahead_behind(value: Option<&str>) -> (Option<u64>, Option<u64>) {
    let Some(value) = value else {
        return (None, None);
    };
    let mut parts = value.split_whitespace();
    let behind = parts.next().and_then(|part| part.parse::<u64>().ok());
    let ahead = parts.next().and_then(|part| part.parse::<u64>().ok());
    (ahead, behind)
}

fn summarize_git_status(status: &str) -> (u64, u64, u64, u64) {
    let mut staged = 0_u64;
    let mut unstaged = 0_u64;
    let mut untracked = 0_u64;
    let mut changed = 0_u64;

    for line in status.lines().filter(|line| !line.is_empty()) {
        changed += 1;
        let bytes = line.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ');
        let y = bytes.get(1).copied().unwrap_or(b' ');
        if x == b'?' && y == b'?' {
            untracked += 1;
            continue;
        }
        if x != b' ' {
            staged += 1;
        }
        if y != b' ' {
            unstaged += 1;
        }
    }

    (staged, unstaged, untracked, changed)
}

fn git_command_output<const N: usize>(workspace_root: &Path, args: [&str; N]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .context("failed to execute git command")?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_status_inspector_rows(status: GitStatusResource) -> Vec<InspectorRow> {
    let mut rows = vec![
        InspectorRow {
            label: "workspace_root".to_string(),
            detail: status.workspace_root,
        },
        InspectorRow {
            label: "git_available".to_string(),
            detail: status.git_available.to_string(),
        },
        InspectorRow {
            label: "repo".to_string(),
            detail: status.repo.to_string(),
        },
        InspectorRow {
            label: "gh_available".to_string(),
            detail: status.gh_available.to_string(),
        },
        InspectorRow {
            label: "branch".to_string(),
            detail: status.branch.unwrap_or_else(|| "unknown".to_string()),
        },
        InspectorRow {
            label: "upstream".to_string(),
            detail: status.upstream.unwrap_or_else(|| "none".to_string()),
        },
        InspectorRow {
            label: "ahead".to_string(),
            detail: status.ahead.unwrap_or_default().to_string(),
        },
        InspectorRow {
            label: "behind".to_string(),
            detail: status.behind.unwrap_or_default().to_string(),
        },
        InspectorRow {
            label: "staged_files".to_string(),
            detail: status.staged_files.to_string(),
        },
        InspectorRow {
            label: "unstaged_files".to_string(),
            detail: status.unstaged_files.to_string(),
        },
        InspectorRow {
            label: "untracked_files".to_string(),
            detail: status.untracked_files.to_string(),
        },
        InspectorRow {
            label: "changed_files".to_string(),
            detail: status.changed_files.to_string(),
        },
        InspectorRow {
            label: "clean".to_string(),
            detail: status.clean.to_string(),
        },
        InspectorRow {
            label: "worktree_active_sessions".to_string(),
            detail: status.worktree_active_sessions.to_string(),
        },
        InspectorRow {
            label: "worktree_managed_dirs".to_string(),
            detail: status.worktree_managed_dirs.to_string(),
        },
    ];
    rows.retain(|row| !row.detail.is_empty());
    rows
}

fn detect_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    match imagesize::blob_size(bytes) {
        Ok(size) => (
            u32::try_from(size.width).ok(),
            u32::try_from(size.height).ok(),
        ),
        Err(_) => (None, None),
    }
}

fn detect_mime(path: &Path, bytes: &[u8]) -> String {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png".to_string();
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return "image/jpeg".to_string();
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return "image/gif".to_string();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }
    if bytes.starts_with(b"BM") {
        return "image/bmp".to_string();
    }
    if bytes.starts_with(b"%PDF-") {
        return "application/pdf".to_string();
    }
    if std::str::from_utf8(bytes).is_ok() {
        return MimeGuess::from_path(path)
            .first_raw()
            .filter(|mime| {
                mime.starts_with("text/")
                    || matches!(
                        *mime,
                        "application/json"
                            | "application/xml"
                            | "application/yaml"
                            | "application/x-yaml"
                            | "application/javascript"
                    )
            })
            .map(str::to_owned)
            .unwrap_or_else(|| "text/plain".to_string());
    }

    MimeGuess::from_path(path)
        .first_raw()
        .map(str::to_owned)
        .unwrap_or_else(|| "application/octet-stream".to_string())
}
