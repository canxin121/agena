use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, OnceLock},
};

use agena::event::{EventFilter, Scope, bus::SubscriptionItem};
use agena::permission::PermissionScope;
use agena::{
    config::{
        ConfigSettingsDeleteInput, ConfigSettingsEditResponse, ConfigSettingsGetInput,
        ConfigSettingsPatchInput, ConfigSettingsSetInput, ProcessEnvironment, ProviderAuthConfig,
        delete_file_setting, draft_atomgit_provider_adapter_models_target,
        draft_gitlab_provider_adapter_models_target, draft_provider_adapter_models_target,
        list_provider_adapter_models_for_target, patch_file_settings, read_file_setting,
        saved_provider_adapter_models_target, set_file_setting,
    },
    event::{DomainEvent, EventKind},
    memory::MemoryStore,
    message::{
        AttachmentItem, AttachmentKind, AttachmentSource, EnterWorktreeToolInput,
        ExitWorktreeToolInput, PartContent, ToolInvocation, UserInputReply,
    },
    model::{AdapterId, ModelRef, ModelSpeedModeRequestOverride},
    permission::PermissionReplyKind,
    provider::ProviderModel,
    provider::auth::{
        AuthData, CredentialIssuer, OAuthUserInfo, exchange_atomgit_oauth_state,
        exchange_gitlab_oauth_code, exchange_openai_oauth_code, parse_oauth_callback_url,
        poll_atomgit_oauth_state, poll_copilot_device_code, start_atomgit_oauth,
        start_copilot_device_code, start_gitlab_oauth, start_openai_browser_oauth,
    },
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

fn resolve_mode_request_override(
    base: &ModelSpeedModeRequestOverride,
    request_override: &ModelSpeedModeRequestOverride,
    adapter_overrides: &std::collections::BTreeMap<String, ModelSpeedModeRequestOverride>,
    resolved_adapter_id: Option<&AdapterId>,
) -> ModelSpeedModeRequestOverride {
    let mut merged = base.merged_with(request_override);
    if let Some(adapter_id) = resolved_adapter_id.map(AdapterId::as_str)
        && let Some(adapter_override) = adapter_overrides.get(adapter_id)
    {
        merged = merged.merged_with(adapter_override);
    }
    merged
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
        ProviderAdapterModelsResource, ProviderAdapterModelsResponse,
        ProviderAdapterSummaryResource, ProviderSummaryResource, RunOptions,
        SessionExecutionResource, SessionResource, WorkspaceResource,
    },
};
use agena_api_server::{
    dispatch,
    local_api::{
        ModelCatalogEntryKind, ModelCatalogEntryResource, ModelCatalogListResponse,
        ModelCatalogResponse as LocalModelCatalogResponse, ModelCatalogSourceKind, normalize_limit,
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
    git_available: bool,
    repo: bool,
    gh_available: bool,
    branch: Option<String>,
    staged_files: u64,
}

#[derive(Debug, Clone)]
pub struct InspectorRow {
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderDraftAuthKind {
    Unset,
    None,
    Api,
    Gitlab,
    Credential(Option<CredentialIssuer>),
    BedrockSigv4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderDraftAdapterRule {
    pub adapter_id: &'static str,
    pub detail: &'static str,
    pub requires_base_url: bool,
    pub supports_draft_model_listing: bool,
}

const NONE_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "ollama",
    detail: "Ollama adapter; auth fields stay empty and the endpoint lives on the adapter.",
    requires_base_url: false,
    supports_draft_model_listing: false,
}];

const API_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai",
        detail: "OpenAI-compatible HTTP adapter; uses provider auth base_url and supports draft model listing.",
        requires_base_url: true,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail: "Anthropic HTTP adapter; uses provider auth base_url and supports draft model listing.",
        requires_base_url: true,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "gemini",
        detail: "Gemini HTTP adapter; uses provider auth base_url and supports draft model listing.",
        requires_base_url: true,
        supports_draft_model_listing: true,
    },
];

const GITLAB_AUTH_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai",
        detail: "GitLab gateway routed through the openai adapter.",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail: "GitLab gateway routed through the anthropic adapter.",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
];

const OPENAI_CHATGPT_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "openai",
    detail: "OpenAI adapter with chatgpt_codex backend; credentials come from the local OpenAI session.",
    requires_base_url: false,
    supports_draft_model_listing: false,
}];

const GITHUB_COPILOT_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai",
        detail: "Copilot token routed through the openai adapter.",
        requires_base_url: false,
        supports_draft_model_listing: false,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail: "Copilot token routed through the anthropic adapter.",
        requires_base_url: false,
        supports_draft_model_listing: false,
    },
];

const GITLAB_CREDENTIAL_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai",
        detail: "GitLab OAuth credential routed through the openai adapter.",
        requires_base_url: false,
        supports_draft_model_listing: false,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail: "GitLab OAuth credential routed through the anthropic adapter.",
        requires_base_url: false,
        supports_draft_model_listing: false,
    },
];

const GOOGLE_ADC_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "openai",
    detail: "Vertex-style openai adapter with capability_family=gemini; requires provider auth base_url.",
    requires_base_url: true,
    supports_draft_model_listing: false,
}];

const SAP_AI_CORE_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "openai",
    detail: "SAP AI Core openai adapter; requires provider auth base_url and service_key_env.",
    requires_base_url: true,
    supports_draft_model_listing: false,
}];

const ATOMGIT_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "openai",
    detail: "AtomGit credential routed through the openai adapter.",
    requires_base_url: false,
    supports_draft_model_listing: true,
}];

const BEDROCK_SIGV4_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "amazon_bedrock",
    detail: "Amazon Bedrock adapter signed with AWS SigV4.",
    requires_base_url: false,
    supports_draft_model_listing: false,
}];

const DEFAULT_LOCAL_OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:1455/callback";
const DEFAULT_GITLAB_INSTANCE_URL: &str = "https://gitlab.com";

impl ProviderDraftAuthKind {
    pub fn label(&self) -> String {
        match self {
            Self::Unset => "unset".to_owned(),
            Self::None => "none".to_owned(),
            Self::Api => "api".to_owned(),
            Self::Gitlab => "gitlab_api".to_owned(),
            Self::Credential(Some(issuer)) => {
                format!("credential:{}", credential_issuer_label(*issuer))
            }
            Self::Credential(None) => "credential".to_owned(),
            Self::BedrockSigv4 => "bedrock_sigv4".to_owned(),
        }
    }

    pub fn supports_draft_model_listing(&self) -> bool {
        matches!(
            self,
            Self::Api | Self::Gitlab | Self::Credential(Some(CredentialIssuer::AtomGit))
        )
    }

    pub fn mode_label(&self) -> &'static str {
        match self {
            Self::Unset => "",
            Self::None => "none",
            Self::Api => "api",
            Self::Gitlab => "gitlab_api",
            Self::Credential(_) => "credential",
            Self::BedrockSigv4 => "bedrock_sigv4",
        }
    }

    pub fn credential_issuer(&self) -> Option<CredentialIssuer> {
        match self {
            Self::Credential(Some(issuer)) => Some(*issuer),
            _ => None,
        }
    }

    pub fn adapter_rules(&self) -> &'static [ProviderDraftAdapterRule] {
        match self {
            Self::Unset => &[],
            Self::None => NONE_ADAPTER_RULES,
            Self::Api => API_ADAPTER_RULES,
            Self::Gitlab => GITLAB_AUTH_ADAPTER_RULES,
            Self::Credential(None) => &[],
            Self::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => OPENAI_CHATGPT_ADAPTER_RULES,
            Self::Credential(Some(CredentialIssuer::GithubCopilot)) => GITHUB_COPILOT_ADAPTER_RULES,
            Self::Credential(Some(CredentialIssuer::Gitlab)) => GITLAB_CREDENTIAL_ADAPTER_RULES,
            Self::Credential(Some(CredentialIssuer::GoogleAdc)) => GOOGLE_ADC_ADAPTER_RULES,
            Self::Credential(Some(CredentialIssuer::SapAiCore)) => SAP_AI_CORE_ADAPTER_RULES,
            Self::Credential(Some(CredentialIssuer::AtomGit)) => ATOMGIT_ADAPTER_RULES,
            Self::BedrockSigv4 => BEDROCK_SIGV4_ADAPTER_RULES,
        }
    }

    pub fn adapter_rule(&self, adapter_id: &str) -> Option<&'static ProviderDraftAdapterRule> {
        let adapter_id = adapter_id.trim();
        self.adapter_rules()
            .iter()
            .find(|rule| rule.adapter_id == adapter_id)
    }

    pub fn supports_adapter(&self, adapter_id: &str) -> bool {
        self.adapter_rule(adapter_id).is_some()
    }

    pub fn parse_mode(value: &str, current_issuer: Option<CredentialIssuer>) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "" => Ok(Self::Unset),
            "none" => Ok(Self::None),
            "api" => Ok(Self::Api),
            "gitlab_api" => Ok(Self::Gitlab),
            "credential" => Ok(Self::Credential(current_issuer)),
            "bedrock_sigv4" => Ok(Self::BedrockSigv4),
            "google_adc" => Ok(Self::Credential(Some(CredentialIssuer::GoogleAdc))),
            "sap_ai_core" => Ok(Self::Credential(Some(CredentialIssuer::SapAiCore))),
            _ if normalized.starts_with("credential:") => {
                let issuer = parse_credential_issuer(
                    normalized
                        .split_once(':')
                        .map(|(_, issuer)| issuer)
                        .unwrap_or_default(),
                )?;
                Ok(Self::Credential(Some(issuer)))
            }
            _ => Err(anyhow!(
                "unsupported auth_mode `{}`; expected none, api, gitlab_api, credential, or bedrock_sigv4",
                value.trim()
            )),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderOAuthTokensDraft {
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at_ms: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderBrowserAuthSessionDraft {
    pub authorize_url: String,
    pub state: String,
    pub pkce_verifier: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderDeviceAuthSessionDraft {
    pub verification_url: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Default)]
pub struct OpenAiChatgptCredentialDraft {
    pub redirect_uri: String,
    pub callback_url: String,
    pub tokens: ProviderOAuthTokensDraft,
    pub account_id: String,
    pub browser: Option<ProviderBrowserAuthSessionDraft>,
}

#[derive(Debug, Clone, Default)]
pub struct GithubCopilotCredentialDraft {
    pub enterprise_domain: String,
    pub tokens: ProviderOAuthTokensDraft,
    pub device: Option<ProviderDeviceAuthSessionDraft>,
}

#[derive(Debug, Clone, Default)]
pub struct GitlabCredentialDraft {
    pub redirect_uri: String,
    pub callback_url: String,
    pub tokens: ProviderOAuthTokensDraft,
    pub browser: Option<ProviderBrowserAuthSessionDraft>,
}

#[derive(Debug, Clone, Default)]
pub struct AtomGitCredentialDraft {
    pub tokens: ProviderOAuthTokensDraft,
    pub account_id: String,
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub avatar_url: String,
    pub browser: Option<ProviderBrowserAuthSessionDraft>,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderCredentialDraftBundle {
    pub openai_chatgpt: OpenAiChatgptCredentialDraft,
    pub github_copilot: GithubCopilotCredentialDraft,
    pub gitlab: GitlabCredentialDraft,
    pub atomgit: AtomGitCredentialDraft,
}

impl ProviderCredentialDraftBundle {
    fn normalize_shape(&mut self) {
        if self.openai_chatgpt.redirect_uri.trim().is_empty() {
            self.openai_chatgpt.redirect_uri = DEFAULT_LOCAL_OAUTH_REDIRECT_URI.to_owned();
        }
        if self.gitlab.redirect_uri.trim().is_empty() {
            self.gitlab.redirect_uri = DEFAULT_LOCAL_OAUTH_REDIRECT_URI.to_owned();
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderDraftAuthActionResult {
    pub draft: ProviderConfigDraft,
    pub message: String,
    pub clipboard_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderConfigDraft {
    pub source_provider_id: Option<String>,
    pub provider_id: String,
    pub auth_kind: ProviderDraftAuthKind,
    pub base_url: String,
    pub instance_url: String,
    pub api_key_env: String,
    pub api_key: String,
    pub credential_issuer: String,
    pub region: String,
    pub profile: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub service_key_env: String,
    pub credential_drafts: ProviderCredentialDraftBundle,
    pub default_adapter: String,
    pub default_model: String,
}

impl ProviderConfigDraft {
    pub fn normalize_shape(&mut self) {
        self.credential_drafts.normalize_shape();
        self.credential_issuer = self
            .auth_kind
            .credential_issuer()
            .map(credential_issuer_label)
            .unwrap_or_default()
            .to_owned();

        match self.auth_kind {
            ProviderDraftAuthKind::Unset => {
                self.base_url.clear();
                self.api_key_env.clear();
                self.api_key.clear();
                self.region.clear();
                self.profile.clear();
                self.access_key_id.clear();
                self.secret_access_key.clear();
                self.session_token.clear();
                self.service_key_env.clear();
            }
            ProviderDraftAuthKind::None => {
                self.base_url.clear();
                self.api_key_env.clear();
                self.api_key.clear();
                self.region.clear();
                self.profile.clear();
                self.access_key_id.clear();
                self.secret_access_key.clear();
                self.session_token.clear();
                self.service_key_env.clear();
            }
            ProviderDraftAuthKind::Api => {
                self.region.clear();
                self.profile.clear();
                self.access_key_id.clear();
                self.secret_access_key.clear();
                self.session_token.clear();
                self.service_key_env.clear();
            }
            ProviderDraftAuthKind::Gitlab => {
                self.base_url.clear();
                self.region.clear();
                self.profile.clear();
                self.access_key_id.clear();
                self.secret_access_key.clear();
                self.session_token.clear();
                self.service_key_env.clear();
                if self.instance_url.trim().is_empty() {
                    self.instance_url = DEFAULT_GITLAB_INSTANCE_URL.to_owned();
                }
            }
            ProviderDraftAuthKind::Credential(None) => {
                self.base_url.clear();
                self.api_key_env.clear();
                self.api_key.clear();
                self.region.clear();
                self.profile.clear();
                self.access_key_id.clear();
                self.secret_access_key.clear();
                self.session_token.clear();
                self.service_key_env.clear();
            }
            ProviderDraftAuthKind::Credential(Some(issuer)) => {
                self.api_key_env.clear();
                self.api_key.clear();
                self.region.clear();
                self.profile.clear();
                self.access_key_id.clear();
                self.secret_access_key.clear();
                self.session_token.clear();
                if !issuer.uses_http_endpoint() {
                    self.base_url.clear();
                }
                if issuer == CredentialIssuer::Gitlab && self.instance_url.trim().is_empty() {
                    self.instance_url = DEFAULT_GITLAB_INSTANCE_URL.to_owned();
                }
                if issuer.requires_service_key_env() {
                    if self.service_key_env.trim().is_empty() {
                        self.service_key_env = "AICORE_SERVICE_KEY".to_owned();
                    }
                } else {
                    self.service_key_env.clear();
                }
            }
            ProviderDraftAuthKind::BedrockSigv4 => {
                self.api_key_env.clear();
                self.api_key.clear();
                self.service_key_env.clear();
            }
        }

        if !self.default_adapter.trim().is_empty()
            && !self
                .auth_kind
                .supports_adapter(self.default_adapter.as_str())
        {
            self.default_adapter.clear();
        }
        if self.default_adapter.trim().is_empty() {
            self.default_model.clear();
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigJsonSources {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub file: JsonValue,
    pub effective: JsonValue,
}

/// Push notification emitted by the unified bus for the active session.
/// Indicates whether the change requires reloading messages.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    /// Concrete event payload when the subscriber kept up with the bus.
    /// `None` means the receiver lagged and the UI should force-refresh
    /// from persisted state instead of trying to apply an incremental patch.
    pub event: Option<DomainEvent>,
    /// True for events that materially change session state — the UI should
    /// trigger a `refresh_session` after handling.
    pub triggers_refresh: bool,
    /// True when the UI should ignore incremental assumptions and force a
    /// replay from persisted state (for example after bus lag).
    pub force_refresh: bool,
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

    pub fn list_agent_names(&self) -> Vec<String> {
        let snapshot = self.runtime.current_snapshot();
        let mut names = snapshot.agents().names();
        names.sort();
        names
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

    pub fn default_adapter_options(&self) -> Vec<ProviderAdapterSummaryResource> {
        let snapshot = self.runtime.current_snapshot();
        let config = &snapshot.config_resolution().config;
        let Some(provider_id) = config.default.provider.as_deref() else {
            return Vec::new();
        };
        let Some(provider) = config.providers.get(provider_id) else {
            return Vec::new();
        };
        if !provider.enabled {
            return Vec::new();
        }

        let mut adapters = provider
            .adapters
            .iter()
            .filter(|(_, adapter)| adapter.enabled)
            .map(|(adapter_id, _)| ProviderAdapterSummaryResource {
                adapter_id: adapter_id.clone(),
                enabled: true,
                configured_model_count: provider
                    .models
                    .iter()
                    .filter(|(route, model)| {
                        model.enabled
                            && route
                                .split_once('/')
                                .map(|(route_adapter_id, _)| route_adapter_id == adapter_id)
                                .unwrap_or(false)
                    })
                    .count(),
            })
            .collect::<Vec<_>>();
        adapters.sort_by(|left, right| left.adapter_id.cmp(&right.adapter_id));
        adapters
    }

    pub fn default_model_options(&self) -> Vec<ProviderModel> {
        let snapshot = self.runtime.current_snapshot();
        let config = &snapshot.config_resolution().config;
        let Some(provider_id) = config.default.provider.as_deref() else {
            return Vec::new();
        };
        let Some(provider) = config.providers.get(provider_id) else {
            return Vec::new();
        };
        if !provider.enabled {
            return Vec::new();
        }
        let adapter_id = config
            .default
            .adapter
            .as_deref()
            .unwrap_or(provider.default_adapter.as_str());
        if !provider
            .adapters
            .get(adapter_id)
            .is_some_and(|adapter| adapter.enabled)
        {
            return Vec::new();
        }

        let mut models = provider
            .models
            .iter()
            .filter_map(|(route, configured)| {
                if !configured.enabled {
                    return None;
                }
                let (route_adapter_id, model_id) = route.split_once('/')?;
                if route_adapter_id != adapter_id {
                    return None;
                }
                let mut model =
                    ProviderModel::new(provider_id, model_id).with_adapter_id(adapter_id);
                if let Some(display_name) = configured
                    .definition
                    .display_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    model = model.with_display_name(display_name);
                }
                Some(model)
            })
            .collect::<Vec<_>>();
        models.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        models
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
        let effective = serde_json::to_value(&resolution.config)
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to serialize effective config")?;
        Ok(ConfigJsonSources {
            config_path,
            config_found: resolution.meta.config_found,
            file,
            effective,
        })
    }

    pub async fn set_config_setting(
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
                dry_run: false,
                validate: true,
                reload: true,
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

    pub async fn delete_config_setting(&self, path: &str) -> Result<ConfigSettingsEditResponse> {
        let config_path = self.runtime.config_resolution().meta.config_path.clone();
        let response = delete_file_setting(
            config_path,
            ConfigSettingsDeleteInput {
                path: path.trim().to_owned(),
                dry_run: false,
                validate: true,
                reload: true,
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

    pub fn provider_config_draft(&self, provider_id: Option<&str>) -> Result<ProviderConfigDraft> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            let mut draft = ProviderConfigDraft {
                source_provider_id: None,
                provider_id: String::new(),
                auth_kind: ProviderDraftAuthKind::Unset,
                base_url: String::new(),
                instance_url: String::new(),
                api_key_env: String::new(),
                api_key: String::new(),
                credential_issuer: String::new(),
                region: String::new(),
                profile: String::new(),
                access_key_id: String::new(),
                secret_access_key: String::new(),
                session_token: String::new(),
                service_key_env: String::new(),
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: String::new(),
                default_model: String::new(),
            };
            draft.normalize_shape();
            return Ok(draft);
        };

        let snapshot = self.runtime.current_snapshot();
        let provider = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow!("provider not found: {provider_id}"))?;

        let uses_legacy_gitlab_adapter = provider.adapters.contains_key("gitlab");
        let mut credential_drafts = ProviderCredentialDraftBundle::default();
        let (
            auth_kind,
            base_url,
            instance_url,
            api_key_env,
            api_key,
            credential_issuer,
            region,
            profile,
            access_key_id,
            secret_access_key,
            session_token,
            service_key_env,
        ) = match &provider.auth {
            ProviderAuthConfig::Api(api) => {
                let auth_kind = if uses_legacy_gitlab_adapter {
                    ProviderDraftAuthKind::Gitlab
                } else {
                    ProviderDraftAuthKind::Api
                };
                (
                    auth_kind,
                    api.base_url.clone().unwrap_or_default(),
                    String::new(),
                    api.api_key_env.clone().unwrap_or_default(),
                    api.api_key.clone().unwrap_or_default(),
                    credential_issuer_label(CredentialIssuer::OpenaiChatgpt).to_owned(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                )
            }
            ProviderAuthConfig::Gitlab(config) => (
                ProviderDraftAuthKind::Gitlab,
                String::new(),
                config.instance_url.clone().unwrap_or_default(),
                config.api_key_env.clone().unwrap_or_default(),
                config.api_key.clone().unwrap_or_default(),
                credential_issuer_label(CredentialIssuer::OpenaiChatgpt).to_owned(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            ProviderAuthConfig::None => (
                ProviderDraftAuthKind::None,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                credential_issuer_label(CredentialIssuer::OpenaiChatgpt).to_owned(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            ProviderAuthConfig::Credential(config) => {
                populate_provider_credential_drafts(
                    &mut credential_drafts,
                    config.issuer,
                    config.credential.as_ref(),
                );
                (
                    ProviderDraftAuthKind::Credential(Some(config.issuer)),
                    config.base_url.clone().unwrap_or_default(),
                    config.instance_url.clone().unwrap_or_default(),
                    String::new(),
                    String::new(),
                    credential_issuer_label(config.issuer).to_owned(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    config.service_key_env.clone().unwrap_or_default(),
                )
            }
            ProviderAuthConfig::BedrockSigv4(sigv4) => (
                ProviderDraftAuthKind::BedrockSigv4,
                sigv4.base_url.clone(),
                String::new(),
                String::new(),
                String::new(),
                credential_issuer_label(CredentialIssuer::OpenaiChatgpt).to_owned(),
                sigv4.region.clone(),
                sigv4.profile.clone().unwrap_or_default(),
                sigv4.access_key_id.clone().unwrap_or_default(),
                sigv4.secret_access_key.clone().unwrap_or_default(),
                sigv4.session_token.clone().unwrap_or_default(),
                String::new(),
            ),
        };

        let mut draft = ProviderConfigDraft {
            source_provider_id: Some(provider_id.to_owned()),
            provider_id: provider_id.to_owned(),
            auth_kind,
            base_url,
            instance_url,
            api_key_env,
            api_key,
            credential_issuer,
            region,
            profile,
            access_key_id,
            secret_access_key,
            session_token,
            service_key_env,
            credential_drafts,
            default_adapter: provider.default_adapter.clone(),
            default_model: provider.default_model.clone(),
        };
        draft.normalize_shape();
        Ok(draft)
    }

    pub async fn start_provider_draft_auth(
        &self,
        draft: ProviderConfigDraft,
    ) -> Result<ProviderDraftAuthActionResult> {
        start_provider_draft_auth(draft).await
    }

    pub async fn continue_provider_draft_auth(
        &self,
        draft: ProviderConfigDraft,
    ) -> Result<ProviderDraftAuthActionResult> {
        continue_provider_draft_auth(draft).await
    }

    fn configured_provider_adapter_ids(
        &self,
        provider_id: Option<&str>,
    ) -> std::collections::BTreeSet<String> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return std::collections::BTreeSet::new();
        };
        let snapshot = self.runtime.current_snapshot();
        snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
            .map(|provider| provider.adapters.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn configured_provider_model_routes(
        &self,
        provider_id: Option<&str>,
    ) -> Vec<(String, String)> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Vec::new();
        };
        let snapshot = self.runtime.current_snapshot();
        let Some(provider) = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
        else {
            return Vec::new();
        };
        provider
            .models
            .keys()
            .filter_map(|route| {
                route
                    .split_once('/')
                    .map(|(adapter_id, model_id)| (adapter_id.to_owned(), model_id.to_owned()))
            })
            .collect()
    }

    pub fn configured_provider_adapter_models(
        &self,
        provider_id: Option<&str>,
    ) -> Vec<ProviderAdapterModelsResource> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Vec::new();
        };
        let snapshot = self.runtime.current_snapshot();
        let Some(provider) = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
        else {
            return Vec::new();
        };

        let mut adapter_ids = provider.adapters.keys().cloned().collect::<Vec<_>>();
        adapter_ids.sort();
        adapter_ids
            .into_iter()
            .map(|adapter_id| {
                let mut model_ids = provider
                    .models
                    .keys()
                    .filter_map(|route| {
                        route
                            .split_once('/')
                            .and_then(|(route_adapter_id, model_id)| {
                                (route_adapter_id == adapter_id).then(|| model_id.to_owned())
                            })
                    })
                    .collect::<Vec<_>>();
                model_ids.sort();
                ProviderAdapterModelsResource {
                    adapter_id: adapter_id.clone(),
                    enabled: provider
                        .adapters
                        .get(adapter_id.as_str())
                        .map(|adapter| adapter.enabled)
                        .unwrap_or(true),
                    resolved_base_url: None,
                    models: model_ids
                        .into_iter()
                        .map(|model_id| ProviderModel::new(adapter_id.as_str(), model_id))
                        .collect(),
                    error: None,
                }
            })
            .collect()
    }

    fn effective_provider_draft_adapter_ids(
        &self,
        draft: &ProviderConfigDraft,
        extra_adapter_ids: &[String],
    ) -> std::collections::BTreeSet<String> {
        let mut adapter_ids =
            self.configured_provider_adapter_ids(draft.source_provider_id.as_deref());
        adapter_ids.extend(
            extra_adapter_ids
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        );
        if let Some(default_adapter) = optional_non_empty(draft.default_adapter.as_str()) {
            adapter_ids.insert(default_adapter.to_owned());
        }
        adapter_ids
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

    pub fn resolved_model_for_run_options(&self, request: &RunOptions) -> Result<ModelRef> {
        if let Some(model) = request.model.as_ref() {
            return Ok(model.clone());
        }

        let snapshot = self.runtime.current_snapshot();
        let registry = snapshot.provider_registry();
        let default_config = &snapshot.config_resolution().config.default;
        if let Some(provider_id) = default_config.provider.as_deref() {
            return registry
                .resolve_model_selection(
                    provider_id,
                    default_config.adapter.as_deref(),
                    default_config.model.as_deref(),
                )
                .context("failed to resolve default model selection");
        }

        let mut providers = registry.provider_ids();
        providers.sort();
        let provider_id = providers
            .first()
            .ok_or_else(|| anyhow!("no providers configured"))?;
        registry
            .resolve_model_target(provider_id, None)
            .context("failed to resolve default provider model")
    }

    pub fn runtime_thinking_mode_rows(&self, request: &RunOptions) -> Result<Vec<InspectorRow>> {
        let snapshot = self.runtime.current_snapshot();
        let registry = snapshot.provider_registry();
        let model = self.resolved_model_for_run_options(request)?;
        let mut rows = registry
            .model_thinking_modes(&model)
            .context("failed to resolve thinking modes for current model")?
            .into_iter()
            .map(|(name, mode)| InspectorRow {
                label: name,
                detail: summarize_named_mode(
                    mode.display_name.as_deref(),
                    mode.description.as_deref(),
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(rows)
    }

    pub fn runtime_speed_mode_rows(&self, request: &RunOptions) -> Result<Vec<InspectorRow>> {
        let snapshot = self.runtime.current_snapshot();
        let registry = snapshot.provider_registry();
        let model = self.resolved_model_for_run_options(request)?;
        let mut rows = registry
            .model_speed_modes(&model)
            .context("failed to resolve speed modes for current model")?
            .into_iter()
            .map(|(name, mode)| InspectorRow {
                label: name,
                detail: summarize_named_mode(
                    mode.display_name.as_deref(),
                    mode.description.as_deref(),
                ),
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(rows)
    }

    pub fn runtime_verbosity_values(&self, request: &RunOptions) -> Result<Vec<String>> {
        let snapshot = self.runtime.current_snapshot();
        let registry = snapshot.provider_registry();
        let model = self.resolved_model_for_run_options(request)?;
        let metadata = registry
            .model_metadata(&model)
            .context("failed to resolve verbosity metadata for current model")?;
        Ok(metadata.supported_verbosity_levels_for_model(&model.model_id))
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

    pub async fn list_draft_provider_adapter_models(
        &self,
        draft: &ProviderConfigDraft,
        adapter_ids: &[String],
    ) -> Result<ProviderAdapterModelsResponse> {
        let mut draft = draft.clone();
        draft.normalize_shape();
        if !draft.auth_kind.supports_draft_model_listing() {
            return Err(anyhow!(
                "draft adapter model listing requires api, gitlab_api, or atomgit credential auth; current auth is {}",
                draft.auth_kind.label()
            ));
        }
        validate_provider_draft_listing_request(&draft, adapter_ids)?;
        let target = match draft.auth_kind {
            ProviderDraftAuthKind::Api => draft_provider_adapter_models_target(
                Some(draft.provider_id.as_str()),
                draft.base_url.as_str(),
                agena::config::ProviderProtocolPathsConfig::default(),
                Some(draft.api_key.as_str()),
                Some(draft.api_key_env.as_str()),
                adapter_ids,
            ),
            ProviderDraftAuthKind::Gitlab => draft_gitlab_provider_adapter_models_target(
                Some(draft.provider_id.as_str()),
                Some(draft.api_key.as_str()),
                Some(draft.api_key_env.as_str()),
                adapter_ids,
            ),
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
                let credential = provider_draft_oauth_auth_data(&draft)?
                    .ok_or_else(|| {
                        anyhow!(
                            "draft atomgit model listing requires OAuth tokens; run AtomGit auth first or enter tokens manually"
                        )
                    })?;
                if !auth_data_has_access_or_api_key(&credential) {
                    return Err(anyhow!(
                        "draft atomgit model listing requires a non-empty access token; run AtomGit auth first or enter access_token manually"
                    ));
                }
                draft_atomgit_provider_adapter_models_target(
                    Some(draft.provider_id.as_str()),
                    credential,
                    adapter_ids,
                )
            }
            _ => unreachable!("listing guard ensures only supported draft auth kinds reach here"),
        }
        .map_err(map_provider_adapter_models_config_error)?;
        self.list_provider_adapter_models_with_target(target).await
    }

    pub async fn list_saved_provider_adapter_models(
        &self,
        provider_id: &str,
        adapter_ids: &[String],
    ) -> Result<ProviderAdapterModelsResponse> {
        let provider_id = provider_id.trim();
        let snapshot = self.runtime.current_snapshot();
        let resolved = snapshot
            .config_resolution()
            .config
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow!("provider not found: {provider_id}"))?;
        let target = saved_provider_adapter_models_target(provider_id, resolved, adapter_ids)
            .map_err(map_provider_adapter_models_config_error)?;
        self.list_provider_adapter_models_with_target(target).await
    }

    pub async fn save_provider_draft(
        &self,
        draft: ProviderConfigDraft,
        adapter_model_lists: &[ProviderAdapterModelsResource],
        selected_adapter_ids: &[String],
        selected_model_keys: &std::collections::BTreeSet<String>,
    ) -> Result<String> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_trimmed(draft.provider_id.as_str(), "provider_id")?;
        let requested_default_adapter =
            optional_non_empty(draft.default_adapter.as_str()).map(str::to_owned);
        let requested_default_model =
            optional_non_empty(draft.default_model.as_str()).map(str::to_owned);
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, selected_adapter_ids);
        validate_provider_draft_shape(&draft, &effective_adapter_ids)?;

        let catalog_entries = self.lookup_model_catalog_entries(
            &adapter_model_lists
                .iter()
                .flat_map(|adapter_models| {
                    adapter_models
                        .models
                        .iter()
                        .map(catalog_lookup_id_for_provider_model)
                })
                .chain(requested_default_model.iter().cloned())
                .collect::<Vec<_>>(),
        );
        let selected = selected_adapter_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<std::collections::BTreeSet<_>>();

        let mut provider_value = self
            .read_file_provider_settings(provider_id)?
            .unwrap_or_else(|| JsonValue::Object(JsonMap::new()));
        let provider_object = provider_value
            .as_object_mut()
            .ok_or_else(|| anyhow!("existing provider settings must be a TOML table"))?;
        let mut adapters = provider_object
            .remove("adapters")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();

        for adapter_models in adapter_model_lists {
            let adapter_id = adapter_models.adapter_id.as_str();
            if adapter_models.error.is_some() || !selected.contains(adapter_id) {
                continue;
            }

            let mut adapter_value = adapters
                .remove(adapter_id)
                .unwrap_or_else(|| JsonValue::Object(JsonMap::new()));
            let adapter_object = adapter_value
                .as_object_mut()
                .ok_or_else(|| anyhow!("provider adapter `{adapter_id}` must be a TOML table"))?;
            let configured_models = adapter_models
                .models
                .iter()
                .filter(|model| {
                    provider_model_selection_contains(
                        selected_model_keys,
                        adapter_id,
                        model.id.as_str(),
                    )
                })
                .map(|model| {
                    (
                        model.id.to_string(),
                        provider_model_json_for_model_id(
                            &catalog_entries,
                            model.id.as_str(),
                            Some(model),
                        ),
                    )
                })
                .collect::<JsonMap<_, _>>();
            adapter_object.insert("enabled".to_owned(), JsonValue::Bool(true));
            adapter_object.insert("models".to_owned(), JsonValue::Object(configured_models));
            adapters.insert(adapter_id.to_owned(), adapter_value);
        }

        let (default_adapter, default_model) = resolve_provider_defaults_from_value(
            &adapters,
            requested_default_adapter.as_deref(),
            requested_default_model.as_deref(),
        )?;

        let default_provider_model = adapter_model_lists
            .iter()
            .find(|adapter_models| adapter_models.adapter_id == default_adapter)
            .and_then(|adapter_models| {
                adapter_models
                    .models
                    .iter()
                    .find(|model| model.id.as_str() == default_model)
                    .cloned()
            });
        let default_model_value = provider_model_json_for_model_id(
            &catalog_entries,
            default_model.as_str(),
            default_provider_model.as_ref(),
        );
        adapters
            .entry(default_adapter.clone())
            .or_insert_with(|| json!({ "enabled": true }));
        ensure_provider_model_entry(
            adapters
                .get_mut(default_adapter.as_str())
                .expect("default adapter must exist"),
            default_model.as_str(),
            default_model_value,
        )?;

        provider_object.insert("enabled".to_owned(), JsonValue::Bool(true));
        provider_object.insert(
            "default_adapter".to_owned(),
            JsonValue::String(default_adapter.clone()),
        );
        provider_object.insert(
            "default_model".to_owned(),
            JsonValue::String(default_model.clone()),
        );
        provider_object.insert(
            "auth".to_owned(),
            JsonValue::Object(build_provider_auth_patch_value(&draft)?),
        );
        provider_object.insert("adapters".to_owned(), JsonValue::Object(adapters));
        self.set_provider_settings(provider_id, provider_value)
            .await?;
        Ok(format!(
            "Saved provider {provider_id} with default {default_adapter}/{default_model}."
        ))
    }

    pub async fn save_provider_adapter_matches(
        &self,
        draft: ProviderConfigDraft,
        adapter_models: ProviderAdapterModelsResource,
    ) -> Result<String> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_trimmed(draft.provider_id.as_str(), "provider_id")?;
        let adapter_id = required_trimmed(adapter_models.adapter_id.as_str(), "adapter_id")?;
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, &[adapter_id.to_owned()]);
        validate_provider_draft_shape(&draft, &effective_adapter_ids)?;
        let catalog_entries = self.lookup_model_catalog_entries(
            &adapter_models
                .models
                .iter()
                .map(catalog_lookup_id_for_provider_model)
                .collect::<Vec<_>>(),
        );
        let configured_models = adapter_models
            .models
            .iter()
            .map(|model| {
                (
                    model.id.to_string(),
                    provider_model_json_for_model_id(
                        &catalog_entries,
                        model.id.as_str(),
                        Some(model),
                    ),
                )
            })
            .collect::<JsonMap<_, _>>();
        let matched_model_count = adapter_models
            .models
            .iter()
            .filter(|model| {
                preferred_catalog_entry_for_provider_model(&catalog_entries, model).is_some()
            })
            .count();
        let provider_patch = build_provider_patch_value(
            &draft,
            optional_non_empty(draft.default_adapter.as_str()).unwrap_or(adapter_id),
            optional_non_empty(draft.default_model.as_str()).unwrap_or("default"),
            json!({
                adapter_id: {
                    "enabled": true,
                    "models": configured_models,
                }
            }),
            false,
        )?;
        self.patch_provider_settings(provider_id, provider_patch)
            .await?;
        Ok(format!(
            "Saved {provider_id}/{adapter_id} with {} listed model(s); {matched_model_count} catalog matched.",
            adapter_models.models.len()
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
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_trimmed(draft.provider_id.as_str(), "provider_id")?;
        let adapter_id = required_trimmed(adapter_id, "adapter_id")?;
        let model_id = required_trimmed(model_id, "model_id")?;
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, &[adapter_id.to_owned()]);
        validate_provider_draft_shape(&draft, &effective_adapter_ids)?;
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

    pub fn provider_model_draft_value(
        &self,
        draft: &ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        provider_model: Option<&ProviderModel>,
    ) -> Result<JsonValue> {
        let adapter_id = required_trimmed(adapter_id, "adapter_id")?;
        let model_id = required_trimmed(model_id, "model_id")?;
        if let Some(provider_id) = draft.source_provider_id.as_deref() {
            let path = provider_model_settings_path(provider_id, adapter_id, model_id);
            let configured = read_file_setting(
                self.runtime.config_resolution().meta.config_path.clone(),
                ConfigSettingsGetInput {
                    path: Some(path),
                    source: agena::config::ConfigSettingsSource::File,
                },
            )
            .map_err(|error| anyhow!(error.to_string()))
            .context("failed to read configured provider model")?
            .value;
            if !configured.is_null() {
                return Ok(configured);
            }
        }

        let catalog_entries = self.lookup_model_catalog_entries(
            &[model_id.to_owned()]
                .into_iter()
                .chain(provider_model.map(catalog_lookup_id_for_provider_model))
                .collect::<Vec<_>>(),
        );
        Ok(provider_model_json_for_model_id(
            &catalog_entries,
            model_id,
            provider_model,
        ))
    }

    fn read_file_provider_settings(&self, provider_id: &str) -> Result<Option<JsonValue>> {
        let configured = read_file_setting(
            self.runtime.config_resolution().meta.config_path.clone(),
            ConfigSettingsGetInput {
                path: Some(provider_settings_path(provider_id)),
                source: agena::config::ConfigSettingsSource::File,
            },
        )
        .map_err(|error| anyhow!(error.to_string()))
        .context("failed to read configured provider")?
        .value;
        if configured.is_null() {
            Ok(None)
        } else {
            Ok(Some(configured))
        }
    }

    pub async fn save_provider_model_value(
        &self,
        draft: ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        model_value: JsonValue,
        set_default: bool,
    ) -> Result<String> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_trimmed(draft.provider_id.as_str(), "provider_id")?;
        let adapter_id = required_trimmed(adapter_id, "adapter_id")?;
        let model_id = required_trimmed(model_id, "model_id")?;
        let JsonValue::Object(_) = &model_value else {
            return Err(anyhow!("provider model config must be a JSON object"));
        };
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, &[adapter_id.to_owned()]);
        validate_provider_draft_shape(&draft, &effective_adapter_ids)?;
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
        Ok(format!(
            "Saved configured model {provider_id}/{adapter_id}/{model_id}."
        ))
    }

    async fn list_provider_adapter_models_with_target(
        &self,
        target: agena::config::ProviderAdapterModelsTarget,
    ) -> Result<ProviderAdapterModelsResponse> {
        let client = agena::provider::ProviderRegistry::build_http_client(
            self.runtime
                .config_resolution()
                .config
                .provider_http_client_config(),
        )
        .context("failed to build provider adapter models http client")?;
        let adapter_models =
            list_provider_adapter_models_for_target(&target, client, &ProcessEnvironment).await;
        Ok(ProviderAdapterModelsResponse {
            provider_id: target.provider_id,
            adapters: adapter_models.into_iter().map(Into::into).collect(),
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
        .map_err(|error| anyhow!("failed to patch provider settings: {error}"))?;

        if response.reload_required {
            self.runtime
                .reload()
                .await
                .context("failed to reload runtime after provider settings change")?;
        }
        Ok(response)
    }

    async fn set_provider_settings(
        &self,
        provider_id: &str,
        provider_value: JsonValue,
    ) -> Result<ConfigSettingsEditResponse> {
        let config_path = self.runtime.config_resolution().meta.config_path.clone();
        let response = set_file_setting(
            config_path,
            ConfigSettingsSetInput {
                path: format!("providers.{}", quoted_settings_segment(provider_id)),
                value: provider_value,
                dry_run: false,
                validate: true,
                reload: true,
            },
        )
        .map_err(|error| anyhow!("failed to save provider settings: {error}"))?;

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
                let event = match item {
                    SubscriptionItem::Event(event) => Some((*event).clone()),
                    SubscriptionItem::Lagged(_) => None,
                };
                if event.is_none() {
                    let live = LiveEvent {
                        event: None,
                        triggers_refresh: true,
                        force_refresh: true,
                    };
                    if tx.send(live).is_err() {
                        break;
                    }
                    continue;
                }
                let event = event.expect("event should exist after lag handling");
                let triggers_refresh = matches!(
                    event.kind,
                    EventKind::ToolCallCompleted(_)
                        | EventKind::TurnCompleted(_)
                        | EventKind::TurnAborted(_)
                        | EventKind::SystemNoticeAppended(_)
                        | EventKind::MessageRevised(_)
                        | EventKind::RunFailed(_)
                );
                let live = LiveEvent {
                    event: Some(event),
                    triggers_refresh,
                    force_refresh: false,
                };
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

    pub fn plugin_tui_content_blocks(
        &self,
    ) -> Vec<agena::plugin::PluginTuiContentBlockCatalogItem> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .tui_content_blocks()
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

        if self.runtime.session_manager().is_none() {
            return Ok(GitStatusResource {
                git_available,
                repo: false,
                gh_available,
                branch: None,
                staged_files: 0,
            });
        }

        if !git_available {
            return Ok(GitStatusResource {
                git_available,
                repo: false,
                gh_available,
                branch: None,
                staged_files: 0,
            });
        }

        let repo = git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]);
        if !repo {
            return Ok(GitStatusResource {
                git_available,
                repo,
                gh_available,
                branch: None,
                staged_files: 0,
            });
        }

        let branch = git_command_output(&workspace_root, ["branch", "--show-current"])?;
        let status = git_command_output(&workspace_root, ["status", "--porcelain"])?;
        let (staged_files, _, _, _) = summarize_git_status(status.as_str());

        Ok(GitStatusResource {
            git_available,
            repo,
            gh_available,
            branch: non_empty(Some(branch.as_str())).map(ToOwned::to_owned),
            staged_files,
        })
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
            thinking_mode,
            speed_mode,
            verbosity,
            parallel_tool_calls,
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
        let resolved_adapter_id = options.model.adapter_id.clone().or_else(|| {
            let snapshot = self.runtime.current_snapshot();
            let provider_registry = snapshot.provider_registry();
            provider_registry
                .get(options.model.provider_id.as_str())
                .and_then(|provider| provider.default_adapter().cloned())
        });

        if let Some(thinking_mode) = thinking_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let snapshot = self.runtime.current_snapshot();
            let provider_registry = snapshot.provider_registry();
            let thinking_modes = provider_registry
                .model_thinking_modes(&options.model)
                .context("failed to resolve selected model thinking modes")?;
            let definition = thinking_modes.get(thinking_mode).ok_or_else(|| {
                anyhow!(
                    "model {} has no thinking mode {thinking_mode}",
                    options.model
                )
            })?;
            options.thinking_mode = Some(thinking_mode.to_string());
            options.thinking = definition.thinking.clone();
            options.request_override = resolve_mode_request_override(
                &options.request_override,
                &definition.request_override,
                &definition.adapter_overrides,
                resolved_adapter_id.as_ref(),
            );
        }
        if let Some(speed_mode) = speed_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let snapshot = self.runtime.current_snapshot();
            let provider_registry = snapshot.provider_registry();
            let speed_modes = provider_registry
                .model_speed_modes(&options.model)
                .context("failed to resolve selected model speed modes")?;
            let definition = speed_modes
                .get(speed_mode)
                .ok_or_else(|| anyhow!("model {} has no speed mode {speed_mode}", options.model))?;
            options.speed_mode = Some(speed_mode.to_string());
            options.request_override = resolve_mode_request_override(
                &options.request_override,
                &definition.request_override,
                &definition.adapter_overrides,
                resolved_adapter_id.as_ref(),
            );
        }
        if let Some(verbosity) = verbosity
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let snapshot = self.runtime.current_snapshot();
            let provider_registry = snapshot.provider_registry();
            let metadata = provider_registry
                .model_metadata(&options.model)
                .context("failed to resolve selected model verbosity metadata")?;
            if !metadata.supports_verbosity_level_for_model(&options.model.model_id, verbosity) {
                let supported =
                    metadata.supported_verbosity_levels_for_model(&options.model.model_id);
                let supported_text = if supported.is_empty() {
                    "none".to_owned()
                } else {
                    supported.join(", ")
                };
                return Err(anyhow!(
                    "model {} does not support verbosity {verbosity}; supported values: {supported_text}",
                    options.model
                ));
            }
            options.verbosity = Some(verbosity.to_ascii_lowercase());
        }
        if let Some(parallel_tool_calls) = parallel_tool_calls {
            let snapshot = self.runtime.current_snapshot();
            let provider_registry = snapshot.provider_registry();
            let metadata = provider_registry
                .model_metadata(&options.model)
                .context("failed to resolve selected model parallel tool call metadata")?;
            if !metadata.supports_parallel_tool_calls_for_model() {
                return Err(anyhow!(
                    "model {} does not support parallel tool calls",
                    options.model
                ));
            }
            options
                .request_override
                .set_parallel_tool_calls(Some(parallel_tool_calls));
        }

        if let Some(system) = system {
            options.system = Some(system);
        }
        if let Some(temperature) = temperature {
            options.temperature = Some(temperature);
        } else if options.temperature.is_none() {
            let snapshot = self.runtime.current_snapshot();
            let provider_registry = snapshot.provider_registry();
            let metadata = provider_registry
                .model_metadata(&options.model)
                .context("failed to resolve selected model temperature metadata")?;
            options.temperature = metadata.parsed_default_temperature();
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

fn required_trimmed<'a>(value: &'a str, field: &str) -> Result<&'a str> {
    optional_non_empty(value).ok_or_else(|| anyhow!("{field} is required"))
}

fn populate_provider_credential_drafts(
    drafts: &mut ProviderCredentialDraftBundle,
    issuer: CredentialIssuer,
    credential: Option<&AuthData>,
) {
    let Some(AuthData::OAuth {
        refresh,
        access,
        expires_at_ms,
        account_id,
        enterprise_url,
        user,
        ..
    }) = credential
    else {
        return;
    };

    let tokens = ProviderOAuthTokensDraft {
        refresh_token: refresh.clone(),
        access_token: access.clone(),
        expires_at_ms: (*expires_at_ms).to_string(),
    };
    match issuer {
        CredentialIssuer::OpenaiChatgpt => {
            drafts.openai_chatgpt.tokens = tokens;
            drafts.openai_chatgpt.account_id = account_id.clone().unwrap_or_default();
        }
        CredentialIssuer::GithubCopilot => {
            drafts.github_copilot.tokens = tokens;
            drafts.github_copilot.enterprise_domain = enterprise_url.clone().unwrap_or_default();
        }
        CredentialIssuer::Gitlab => {
            drafts.gitlab.tokens = tokens;
        }
        CredentialIssuer::AtomGit => {
            drafts.atomgit.tokens = tokens;
            drafts.atomgit.account_id = account_id
                .clone()
                .or_else(|| user.as_ref().map(|user| user.id.clone()))
                .unwrap_or_default();
            if let Some(user) = user {
                drafts.atomgit.username = user.username.clone();
                drafts.atomgit.display_name = user.name.clone().unwrap_or_default();
                drafts.atomgit.email = user.email.clone().unwrap_or_default();
                drafts.atomgit.avatar_url = user.avatar_url.clone().unwrap_or_default();
            }
        }
        CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore => {}
    }
}

fn update_oauth_tokens_from_response(
    tokens: &mut ProviderOAuthTokensDraft,
    response: &agena::provider::auth::OAuthTokenResponse,
) {
    tokens.refresh_token = response.refresh.clone();
    tokens.access_token = response.access.clone();
    tokens.expires_at_ms = response.expires_at_ms.to_string();
}

async fn start_provider_draft_auth(
    mut draft: ProviderConfigDraft,
) -> Result<ProviderDraftAuthActionResult> {
    draft.normalize_shape();
    match draft.auth_kind {
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            let redirect_uri = required_trimmed(
                draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                "redirect_uri",
            )?;
            let start = start_openai_browser_oauth(redirect_uri)?;
            draft.credential_drafts.openai_chatgpt.callback_url.clear();
            draft.credential_drafts.openai_chatgpt.browser =
                Some(ProviderBrowserAuthSessionDraft {
                    authorize_url: start.authorize_url.clone(),
                    state: start.state.clone(),
                    pkce_verifier: start.pkce_verifier,
                });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: "OpenAI browser auth started. Open the copied authorize URL, then paste the redirected callback URL and press p.".to_owned(),
                clipboard_text: Some(start.authorize_url),
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
            let domain = optional_non_empty(
                draft
                    .credential_drafts
                    .github_copilot
                    .enterprise_domain
                    .as_str(),
            )
            .unwrap_or("github.com");
            let start = start_copilot_device_code(domain).await?;
            draft.credential_drafts.github_copilot.device = Some(ProviderDeviceAuthSessionDraft {
                verification_url: start.verification_url.clone(),
                user_code: start.user_code.clone(),
                device_code: start.device_code,
                interval_seconds: start.interval_seconds,
            });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: format!(
                    "Copilot device login started. Open the copied verification URL, enter code {}, then press p.",
                    start.user_code
                ),
                clipboard_text: Some(start.verification_url),
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            let instance_url = required_trimmed(draft.instance_url.as_str(), "instance_url")?;
            let redirect_uri = required_trimmed(
                draft.credential_drafts.gitlab.redirect_uri.as_str(),
                "redirect_uri",
            )?;
            let start = start_gitlab_oauth(instance_url, redirect_uri)?;
            draft.credential_drafts.gitlab.callback_url.clear();
            draft.credential_drafts.gitlab.browser = Some(ProviderBrowserAuthSessionDraft {
                authorize_url: start.authorize_url.clone(),
                state: start.state.clone(),
                pkce_verifier: start.pkce_verifier,
            });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: "GitLab browser auth started. Open the copied authorize URL, then paste the redirected callback URL and press p.".to_owned(),
                clipboard_text: Some(start.authorize_url),
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
            let start = start_atomgit_oauth().await?;
            draft.credential_drafts.atomgit.browser = Some(ProviderBrowserAuthSessionDraft {
                authorize_url: start.authorize_url.clone(),
                state: start.state,
                pkce_verifier: String::new(),
            });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: "AtomGit browser auth started. Open the copied authorize URL, complete the login, then press p to poll.".to_owned(),
                clipboard_text: Some(start.authorize_url),
            })
        }
        _ => Err(anyhow!(
            "the current auth_mode does not support interactive OAuth login"
        )),
    }
}

async fn continue_provider_draft_auth(
    mut draft: ProviderConfigDraft,
) -> Result<ProviderDraftAuthActionResult> {
    draft.normalize_shape();
    match draft.auth_kind {
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            let session = draft
                .credential_drafts
                .openai_chatgpt
                .browser
                .clone()
                .ok_or_else(|| anyhow!("start browser auth first with o"))?;
            let redirect_uri = required_trimmed(
                draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                "redirect_uri",
            )?;
            let callback_url = required_trimmed(
                draft.credential_drafts.openai_chatgpt.callback_url.as_str(),
                "callback_url",
            )?;
            let callback = parse_oauth_callback_url(callback_url, Some(session.state.as_str()))?;
            let token = exchange_openai_oauth_code(
                callback.code.as_str(),
                session.pkce_verifier.as_str(),
                redirect_uri,
            )
            .await?;
            update_oauth_tokens_from_response(
                &mut draft.credential_drafts.openai_chatgpt.tokens,
                &token,
            );
            draft.credential_drafts.openai_chatgpt.account_id =
                token.account_id.unwrap_or_default();
            draft.credential_drafts.openai_chatgpt.callback_url.clear();
            draft.credential_drafts.openai_chatgpt.browser = None;
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: "OpenAI OAuth credential captured into the draft.".to_owned(),
                clipboard_text: None,
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
            let session = draft
                .credential_drafts
                .github_copilot
                .device
                .clone()
                .ok_or_else(|| anyhow!("start device auth first with o"))?;
            let domain = optional_non_empty(
                draft
                    .credential_drafts
                    .github_copilot
                    .enterprise_domain
                    .as_str(),
            )
            .unwrap_or("github.com");
            let Some(token) =
                poll_copilot_device_code(domain, session.device_code.as_str()).await?
            else {
                return Ok(ProviderDraftAuthActionResult {
                    draft,
                    message: "Copilot device login is still pending. Complete the browser approval, then press p again.".to_owned(),
                    clipboard_text: None,
                });
            };
            update_oauth_tokens_from_response(
                &mut draft.credential_drafts.github_copilot.tokens,
                &token,
            );
            draft.credential_drafts.github_copilot.device = None;
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: "Copilot OAuth credential captured into the draft.".to_owned(),
                clipboard_text: None,
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            let session = draft
                .credential_drafts
                .gitlab
                .browser
                .clone()
                .ok_or_else(|| anyhow!("start browser auth first with o"))?;
            let instance_url = required_trimmed(draft.instance_url.as_str(), "instance_url")?;
            let redirect_uri = required_trimmed(
                draft.credential_drafts.gitlab.redirect_uri.as_str(),
                "redirect_uri",
            )?;
            let callback_url = required_trimmed(
                draft.credential_drafts.gitlab.callback_url.as_str(),
                "callback_url",
            )?;
            let callback = parse_oauth_callback_url(callback_url, Some(session.state.as_str()))?;
            let token = exchange_gitlab_oauth_code(
                instance_url,
                callback.code.as_str(),
                session.pkce_verifier.as_str(),
                redirect_uri,
            )
            .await?;
            update_oauth_tokens_from_response(&mut draft.credential_drafts.gitlab.tokens, &token);
            draft.credential_drafts.gitlab.callback_url.clear();
            draft.credential_drafts.gitlab.browser = None;
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: "GitLab OAuth credential captured into the draft.".to_owned(),
                clipboard_text: None,
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
            let session = draft
                .credential_drafts
                .atomgit
                .browser
                .clone()
                .ok_or_else(|| anyhow!("start browser auth first with o"))?;
            if !poll_atomgit_oauth_state(session.state.as_str()).await? {
                return Ok(ProviderDraftAuthActionResult {
                    draft,
                    message: "AtomGit browser login is still pending. Finish the browser flow, then press p again.".to_owned(),
                    clipboard_text: None,
                });
            }

            let token = exchange_atomgit_oauth_state(session.state.as_str()).await?;
            update_oauth_tokens_from_response(&mut draft.credential_drafts.atomgit.tokens, &token);
            draft.credential_drafts.atomgit.account_id =
                token.account_id.clone().unwrap_or_default();
            if let Some(user) = token.user {
                draft.credential_drafts.atomgit.account_id = user.id.clone();
                draft.credential_drafts.atomgit.username = user.username;
                draft.credential_drafts.atomgit.display_name = user.name.unwrap_or_default();
                draft.credential_drafts.atomgit.email = user.email.unwrap_or_default();
                draft.credential_drafts.atomgit.avatar_url = user.avatar_url.unwrap_or_default();
            }
            draft.credential_drafts.atomgit.browser = None;
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: "AtomGit OAuth credential captured into the draft.".to_owned(),
                clipboard_text: None,
            })
        }
        _ => Err(anyhow!(
            "the current auth_mode does not support interactive OAuth login"
        )),
    }
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
    let thinking_mode_text = entry
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
    let speed_mode_text = entry
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
        entry.model_id.clone(),
        entry.display_name.clone().unwrap_or_default(),
        entry.origin.clone().unwrap_or_default(),
        entry.description.clone().unwrap_or_default(),
        entry
            .context_window_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        entry
            .max_input_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        entry
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
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
        thinking_mode_text,
        speed_mode_text,
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
    if let Some(max_input_tokens) = entry.max_input_tokens {
        value.insert(
            "max_input_tokens".to_owned(),
            JsonValue::Number(max_input_tokens.into()),
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
    if let Some(knowledge_cutoff) = non_empty(entry.knowledge_cutoff.as_deref()) {
        value.insert(
            "knowledge_cutoff".to_owned(),
            JsonValue::String(knowledge_cutoff.to_owned()),
        );
    }
    if let Some(release_date) = non_empty(entry.release_date.as_deref()) {
        value.insert(
            "release_date".to_owned(),
            JsonValue::String(release_date.to_owned()),
        );
    }
    if let Some(last_updated) = non_empty(entry.last_updated.as_deref()) {
        value.insert(
            "last_updated".to_owned(),
            JsonValue::String(last_updated.to_owned()),
        );
    }
    if let Some(open_weights) = entry.open_weights {
        value.insert("open_weights".to_owned(), JsonValue::Bool(open_weights));
    }
    if let Some(default_thinking_mode) = non_empty(entry.default_thinking_mode.as_deref()) {
        value.insert(
            "default_thinking_mode".to_owned(),
            JsonValue::String(default_thinking_mode.to_owned()),
        );
    }
    if let Some(supports_parallel_tool_calls) = entry.supports_parallel_tool_calls {
        value.insert(
            "supports_parallel_tool_calls".to_owned(),
            JsonValue::Bool(supports_parallel_tool_calls),
        );
    }
    if let Some(supports_verbosity) = entry.supports_verbosity {
        value.insert(
            "supports_verbosity".to_owned(),
            JsonValue::Bool(supports_verbosity),
        );
    }
    if let Some(default_verbosity) = non_empty(entry.default_verbosity.as_deref()) {
        value.insert(
            "default_verbosity".to_owned(),
            JsonValue::String(default_verbosity.to_owned()),
        );
    }
    if let Some(default_temperature) = non_empty(entry.default_temperature.as_deref()) {
        value.insert(
            "default_temperature".to_owned(),
            JsonValue::String(default_temperature.to_owned()),
        );
    }
    if let Some(default_top_p) = non_empty(entry.default_top_p.as_deref()) {
        value.insert(
            "default_top_p".to_owned(),
            JsonValue::String(default_top_p.to_owned()),
        );
    }
    if let Some(default_top_k) = entry.default_top_k {
        value.insert(
            "default_top_k".to_owned(),
            JsonValue::Number(default_top_k.into()),
        );
    }
    if let Some(assistant_reasoning_interleaved) = entry.assistant_reasoning_interleaved {
        value.insert(
            "assistant_reasoning_interleaved".to_owned(),
            JsonValue::Bool(assistant_reasoning_interleaved),
        );
    }
    if let Some(assistant_reasoning_field) = non_empty(entry.assistant_reasoning_field.as_deref()) {
        value.insert(
            "assistant_reasoning_field".to_owned(),
            JsonValue::String(assistant_reasoning_field.to_owned()),
        );
    }
    if !entry.output_modalities.is_empty() {
        value.insert(
            "output_modalities".to_owned(),
            json!(entry.output_modalities),
        );
    }
    if let Some(pricing) = entry.pricing.as_ref() {
        value.insert("pricing".to_owned(), json!(pricing));
    }
    if !entry.thinking_modes.is_empty() {
        value.insert("thinking_modes".to_owned(), json!(entry.thinking_modes));
    }
    if !entry.speed_modes.is_empty() {
        value.insert("speed_modes".to_owned(), json!(entry.speed_modes));
    }
    let capabilities_patch = sanitized_catalog_capability_patch(&entry.capabilities);
    if let Ok(JsonValue::Object(capabilities)) = serde_json::to_value(&capabilities_patch) {
        for (key, part) in capabilities {
            let is_empty_object = matches!(&part, JsonValue::Object(object) if object.is_empty());
            if !part.is_null() && !is_empty_object {
                value.insert(key, part);
            }
        }
    }
    JsonValue::Object(value)
}

fn sanitized_catalog_capability_patch(
    patch: &agena::provider::ModelCapabilityPatch,
) -> agena::provider::ModelCapabilityPatch {
    use agena::provider::{
        FeatureCapabilityPatch, FeatureCapabilityPatchBody, InputCapabilityPatch,
        InputCapabilityPatchBody,
    };

    let mut patch = patch.clone();

    match patch.input.take() {
        Some(InputCapabilityPatch::Supported(mut supported)) => {
            dedupe_vec(&mut supported);
            patch.input =
                (!supported.is_empty()).then_some(InputCapabilityPatch::Supported(supported));
        }
        Some(InputCapabilityPatch::Patch(mut values)) => {
            dedupe_vec(&mut values.supported);
            dedupe_vec(&mut values.unsupported);
            values
                .unsupported
                .retain(|value| !values.supported.contains(value));
            patch.input = if values.unsupported.is_empty() {
                (!values.supported.is_empty())
                    .then_some(InputCapabilityPatch::Supported(values.supported))
            } else if values.supported.is_empty() {
                Some(InputCapabilityPatch::Patch(InputCapabilityPatchBody {
                    supported: Vec::new(),
                    unsupported: values.unsupported,
                }))
            } else {
                Some(InputCapabilityPatch::Patch(values))
            };
        }
        None => {}
    }

    match patch.features.take() {
        Some(FeatureCapabilityPatch::Supported(mut supported)) => {
            dedupe_vec(&mut supported);
            patch.features =
                (!supported.is_empty()).then_some(FeatureCapabilityPatch::Supported(supported));
        }
        Some(FeatureCapabilityPatch::Patch(mut values)) => {
            dedupe_vec(&mut values.supported);
            dedupe_vec(&mut values.unsupported);
            values
                .unsupported
                .retain(|value| !values.supported.contains(value));
            patch.features = if values.unsupported.is_empty() {
                (!values.supported.is_empty())
                    .then_some(FeatureCapabilityPatch::Supported(values.supported))
            } else if values.supported.is_empty() {
                Some(FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                    supported: Vec::new(),
                    unsupported: values.unsupported,
                }))
            } else {
                Some(FeatureCapabilityPatch::Patch(values))
            };
        }
        None => {}
    }

    patch
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
    if let Some(max_input_tokens) = model.metadata.limits.max_input_tokens {
        value.insert(
            "max_input_tokens".to_owned(),
            JsonValue::Number(max_input_tokens.into()),
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
    if let Some(knowledge_cutoff) = non_empty(model.metadata.knowledge_cutoff.as_deref()) {
        value.insert(
            "knowledge_cutoff".to_owned(),
            JsonValue::String(knowledge_cutoff.to_owned()),
        );
    }
    if let Some(release_date) = non_empty(model.metadata.release_date.as_deref()) {
        value.insert(
            "release_date".to_owned(),
            JsonValue::String(release_date.to_owned()),
        );
    }
    if let Some(last_updated) = non_empty(model.metadata.last_updated.as_deref()) {
        value.insert(
            "last_updated".to_owned(),
            JsonValue::String(last_updated.to_owned()),
        );
    }
    if let Some(open_weights) = model.metadata.open_weights {
        value.insert("open_weights".to_owned(), JsonValue::Bool(open_weights));
    }
    if let Some(default_thinking_mode) = non_empty(model.metadata.default_thinking_mode.as_deref())
    {
        value.insert(
            "default_thinking_mode".to_owned(),
            JsonValue::String(default_thinking_mode.to_owned()),
        );
    }
    if let Some(supports_parallel_tool_calls) = model.metadata.supports_parallel_tool_calls {
        value.insert(
            "supports_parallel_tool_calls".to_owned(),
            JsonValue::Bool(supports_parallel_tool_calls),
        );
    }
    if let Some(supports_verbosity) = model.metadata.supports_verbosity {
        value.insert(
            "supports_verbosity".to_owned(),
            JsonValue::Bool(supports_verbosity),
        );
    }
    if let Some(default_verbosity) = non_empty(model.metadata.default_verbosity.as_deref()) {
        value.insert(
            "default_verbosity".to_owned(),
            JsonValue::String(default_verbosity.to_owned()),
        );
    }
    if let Some(default_temperature) = non_empty(model.metadata.default_temperature.as_deref()) {
        value.insert(
            "default_temperature".to_owned(),
            JsonValue::String(default_temperature.to_owned()),
        );
    }
    if let Some(default_top_p) = non_empty(model.metadata.default_top_p.as_deref()) {
        value.insert(
            "default_top_p".to_owned(),
            JsonValue::String(default_top_p.to_owned()),
        );
    }
    if let Some(default_top_k) = model.metadata.default_top_k {
        value.insert(
            "default_top_k".to_owned(),
            JsonValue::Number(default_top_k.into()),
        );
    }
    if let Some(assistant_reasoning_interleaved) = model.metadata.assistant_reasoning_interleaved {
        value.insert(
            "assistant_reasoning_interleaved".to_owned(),
            JsonValue::Bool(assistant_reasoning_interleaved),
        );
    }
    if let Some(assistant_reasoning_field) =
        non_empty(model.metadata.assistant_reasoning_field.as_deref())
    {
        value.insert(
            "assistant_reasoning_field".to_owned(),
            JsonValue::String(assistant_reasoning_field.to_owned()),
        );
    }
    if !model.metadata.output_modalities.is_empty() {
        value.insert(
            "output_modalities".to_owned(),
            json!(model.metadata.output_modalities),
        );
    }
    if let Some(pricing) = model.metadata.pricing.as_ref() {
        value.insert("pricing".to_owned(), json!(pricing));
    }
    if !model.thinking_modes.is_empty() {
        let thinking_modes = model
            .thinking_modes
            .iter()
            .map(|(name, mode)| {
                (
                    name.clone(),
                    json!({
                        "display_name": mode.display_name,
                        "description": mode.description,
                        "thinking": mode.thinking,
                        "request_override": mode.request_override,
                        "adapter_overrides": mode.adapter_overrides,
                    }),
                )
            })
            .collect::<JsonMap<String, JsonValue>>();
        value.insert(
            "thinking_modes".to_owned(),
            JsonValue::Object(thinking_modes),
        );
    }
    if !model.speed_modes.is_empty() {
        let speed_modes = model
            .speed_modes
            .iter()
            .map(|(name, mode)| {
                (
                    name.clone(),
                    json!({
                        "display_name": mode.display_name,
                        "description": mode.description,
                        "request_override": mode.request_override,
                        "adapter_overrides": mode.adapter_overrides,
                    }),
                )
            })
            .collect::<JsonMap<String, JsonValue>>();
        value.insert("speed_modes".to_owned(), JsonValue::Object(speed_modes));
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

fn dedupe_vec<T: PartialEq>(values: &mut Vec<T>) {
    let mut index = 0;
    while index < values.len() {
        let mut next = index + 1;
        while next < values.len() {
            if values[index] == values[next] {
                values.remove(next);
            } else {
                next += 1;
            }
        }
        index += 1;
    }
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

fn map_provider_adapter_models_config_error(error: agena::config::ConfigError) -> anyhow::Error {
    match error {
        agena::config::ConfigError::Validation(message)
        | agena::config::ConfigError::App(agena::AppError::Config(message)) => anyhow!(message),
        other => anyhow!(other.to_string()),
    }
}

fn validate_provider_draft_listing_request(
    draft: &ProviderConfigDraft,
    adapter_ids: &[String],
) -> Result<()> {
    let selected = adapter_ids
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(anyhow!(
            "draft adapter model listing requires at least one explicit adapter"
        ));
    }
    let unsupported = selected
        .iter()
        .filter(|adapter_id| {
            draft
                .auth_kind
                .adapter_rule(adapter_id)
                .map(|rule| !rule.supports_draft_model_listing)
                .unwrap_or(true)
        })
        .copied()
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        return Err(anyhow!(
            "draft adapter model listing only supports adapters with live model discovery for the current auth; unsupported: {}",
            unsupported.join(", ")
        ));
    }
    validate_provider_draft_shape(
        draft,
        &selected
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<std::collections::BTreeSet<_>>(),
    )
}

fn validate_provider_draft_shape(
    draft: &ProviderConfigDraft,
    adapter_ids: &std::collections::BTreeSet<String>,
) -> Result<()> {
    let default_adapter = required_trimmed(draft.default_adapter.as_str(), "default_adapter")?;
    if !draft.auth_kind.supports_adapter(default_adapter) {
        return Err(anyhow!(
            "auth {} does not support default_adapter `{default_adapter}`; expected one of {}",
            draft.auth_kind.label(),
            supported_provider_draft_adapter_list(&draft.auth_kind),
        ));
    }

    let incompatible = adapter_ids
        .iter()
        .filter(|adapter_id| !draft.auth_kind.supports_adapter(adapter_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !incompatible.is_empty() {
        return Err(anyhow!(
            "auth {} does not support adapter(s): {}; expected one of {}",
            draft.auth_kind.label(),
            incompatible.join(", "),
            supported_provider_draft_adapter_list(&draft.auth_kind),
        ));
    }

    match draft.auth_kind {
        ProviderDraftAuthKind::Unset => {
            return Err(anyhow!("provider auth_mode is required"));
        }
        ProviderDraftAuthKind::None => {}
        ProviderDraftAuthKind::Api => {
            let requires_base_url = adapter_ids.iter().any(|adapter_id| {
                draft
                    .auth_kind
                    .adapter_rule(adapter_id.as_str())
                    .map(|rule| rule.requires_base_url)
                    .unwrap_or(false)
            });
            if requires_base_url && optional_non_empty(draft.base_url.as_str()).is_none() {
                return Err(anyhow!(
                    "api auth requires base_url when using openai, anthropic, or gemini adapters"
                ));
            }
        }
        ProviderDraftAuthKind::Gitlab => {
            if optional_non_empty(draft.api_key.as_str()).is_none()
                && optional_non_empty(draft.api_key_env.as_str()).is_none()
            {
                return Err(anyhow!("gitlab_api auth requires api_key or api_key_env"));
            }
        }
        ProviderDraftAuthKind::Credential(None) => {
            return Err(anyhow!("credential auth requires credential_issuer"));
        }
        ProviderDraftAuthKind::Credential(Some(issuer)) => {
            if issuer.uses_http_endpoint() && optional_non_empty(draft.base_url.as_str()).is_none()
            {
                return Err(anyhow!(
                    "credential issuer `{}` requires base_url",
                    credential_issuer_label(issuer)
                ));
            }
            if issuer.requires_service_key_env()
                && optional_non_empty(draft.service_key_env.as_str()).is_none()
            {
                return Err(anyhow!(
                    "credential issuer `{}` requires service_key_env",
                    credential_issuer_label(issuer)
                ));
            }
        }
        ProviderDraftAuthKind::BedrockSigv4 => {
            let has_access_key_id = optional_non_empty(draft.access_key_id.as_str()).is_some();
            let has_secret_access_key =
                optional_non_empty(draft.secret_access_key.as_str()).is_some();
            if has_access_key_id ^ has_secret_access_key {
                return Err(anyhow!(
                    "bedrock_sigv4 requires access_key_id and secret_access_key together"
                ));
            }
        }
    }

    Ok(())
}

fn supported_provider_draft_adapter_list(auth_kind: &ProviderDraftAuthKind) -> String {
    let supported = auth_kind
        .adapter_rules()
        .iter()
        .map(|rule| rule.adapter_id)
        .collect::<Vec<_>>()
        .join(", ");
    if supported.is_empty() {
        "no adapters until auth details are selected".to_owned()
    } else {
        supported
    }
}

fn parse_oauth_expires_at_ms(value: &str) -> Result<i64> {
    optional_non_empty(value)
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| anyhow!("expires_at_ms must be an integer"))
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

fn provider_draft_oauth_auth_data(draft: &ProviderConfigDraft) -> Result<Option<AuthData>> {
    match draft.auth_kind {
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            let tokens = &draft.credential_drafts.openai_chatgpt.tokens;
            if optional_non_empty(tokens.refresh_token.as_str()).is_none()
                && optional_non_empty(tokens.access_token.as_str()).is_none()
            {
                return Ok(None);
            }
            Ok(Some(AuthData::OAuth {
                issuer: Some(CredentialIssuer::OpenaiChatgpt),
                refresh: tokens.refresh_token.clone(),
                access: tokens.access_token.clone(),
                expires_at_ms: parse_oauth_expires_at_ms(tokens.expires_at_ms.as_str())?,
                account_id: optional_non_empty(
                    draft.credential_drafts.openai_chatgpt.account_id.as_str(),
                )
                .map(ToOwned::to_owned),
                enterprise_url: None,
                user: None,
            }))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
            let tokens = &draft.credential_drafts.github_copilot.tokens;
            if optional_non_empty(tokens.refresh_token.as_str()).is_none()
                && optional_non_empty(tokens.access_token.as_str()).is_none()
            {
                return Ok(None);
            }
            Ok(Some(AuthData::OAuth {
                issuer: Some(CredentialIssuer::GithubCopilot),
                refresh: tokens.refresh_token.clone(),
                access: tokens.access_token.clone(),
                expires_at_ms: parse_oauth_expires_at_ms(tokens.expires_at_ms.as_str())?,
                account_id: None,
                enterprise_url: optional_non_empty(
                    draft
                        .credential_drafts
                        .github_copilot
                        .enterprise_domain
                        .as_str(),
                )
                .map(ToOwned::to_owned),
                user: None,
            }))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            let tokens = &draft.credential_drafts.gitlab.tokens;
            if optional_non_empty(tokens.refresh_token.as_str()).is_none()
                && optional_non_empty(tokens.access_token.as_str()).is_none()
            {
                return Ok(None);
            }
            Ok(Some(AuthData::OAuth {
                issuer: Some(CredentialIssuer::Gitlab),
                refresh: tokens.refresh_token.clone(),
                access: tokens.access_token.clone(),
                expires_at_ms: parse_oauth_expires_at_ms(tokens.expires_at_ms.as_str())?,
                account_id: None,
                enterprise_url: None,
                user: None,
            }))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
            let tokens = &draft.credential_drafts.atomgit.tokens;
            if optional_non_empty(tokens.refresh_token.as_str()).is_none()
                && optional_non_empty(tokens.access_token.as_str()).is_none()
            {
                return Ok(None);
            }
            let account_id =
                optional_non_empty(draft.credential_drafts.atomgit.account_id.as_str())
                    .map(ToOwned::to_owned);
            let username = optional_non_empty(draft.credential_drafts.atomgit.username.as_str())
                .map(ToOwned::to_owned);
            let user = match (account_id.clone(), username.clone()) {
                (Some(id), Some(username)) => Some(OAuthUserInfo {
                    id,
                    username,
                    name: optional_non_empty(draft.credential_drafts.atomgit.display_name.as_str())
                        .map(ToOwned::to_owned),
                    email: optional_non_empty(draft.credential_drafts.atomgit.email.as_str())
                        .map(ToOwned::to_owned),
                    avatar_url: optional_non_empty(
                        draft.credential_drafts.atomgit.avatar_url.as_str(),
                    )
                    .map(ToOwned::to_owned),
                }),
                (None, None) => None,
                _ => {
                    return Err(anyhow!(
                        "atomgit manual credential requires both account_id and username when storing user metadata"
                    ));
                }
            };
            Ok(Some(AuthData::OAuth {
                issuer: Some(CredentialIssuer::AtomGit),
                refresh: tokens.refresh_token.clone(),
                access: tokens.access_token.clone(),
                expires_at_ms: parse_oauth_expires_at_ms(tokens.expires_at_ms.as_str())?,
                account_id,
                enterprise_url: None,
                user,
            }))
        }
        ProviderDraftAuthKind::Credential(Some(
            CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore,
        ))
        | ProviderDraftAuthKind::Unset
        | ProviderDraftAuthKind::None
        | ProviderDraftAuthKind::Api
        | ProviderDraftAuthKind::Gitlab
        | ProviderDraftAuthKind::Credential(None)
        | ProviderDraftAuthKind::BedrockSigv4 => Ok(None),
    }
}

fn auth_data_has_access_or_api_key(auth: &AuthData) -> bool {
    match auth {
        AuthData::Api { key } | AuthData::WellKnown { key, .. } => !key.trim().is_empty(),
        AuthData::OAuth { access, .. } => !access.trim().is_empty(),
    }
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
    value.insert(
        "auth".to_owned(),
        JsonValue::Object(build_provider_auth_patch_value(draft)?),
    );
    value.insert("adapters".to_owned(), adapters);
    Ok(JsonValue::Object(value))
}

fn build_provider_auth_patch_value(
    draft: &ProviderConfigDraft,
) -> Result<JsonMap<String, JsonValue>> {
    let mut auth = JsonMap::new();
    let inline_credential = provider_draft_oauth_auth_data(draft)?;
    auth.insert(
        "mode".to_owned(),
        JsonValue::String(draft.auth_kind.mode_label().to_owned()),
    );
    match draft.auth_kind {
        ProviderDraftAuthKind::Unset => {
            return Err(anyhow!("provider auth_mode is required before saving"));
        }
        ProviderDraftAuthKind::None => {
            auth.insert("base_url".to_owned(), JsonValue::Null);
            auth.insert("instance_url".to_owned(), JsonValue::Null);
            auth.insert("api_key_env".to_owned(), JsonValue::Null);
            auth.insert("api_key".to_owned(), JsonValue::Null);
            auth.insert("credential".to_owned(), JsonValue::Null);
            auth.insert("issuer".to_owned(), JsonValue::Null);
            auth.insert("region".to_owned(), JsonValue::Null);
            auth.insert("profile".to_owned(), JsonValue::Null);
            auth.insert("access_key_id".to_owned(), JsonValue::Null);
            auth.insert("secret_access_key".to_owned(), JsonValue::Null);
            auth.insert("session_token".to_owned(), JsonValue::Null);
            auth.insert("service_key_env".to_owned(), JsonValue::Null);
        }
        ProviderDraftAuthKind::Api => {
            auth.insert("instance_url".to_owned(), JsonValue::Null);
            auth.insert("issuer".to_owned(), JsonValue::Null);
            auth.insert("region".to_owned(), JsonValue::Null);
            auth.insert("profile".to_owned(), JsonValue::Null);
            auth.insert("access_key_id".to_owned(), JsonValue::Null);
            auth.insert("secret_access_key".to_owned(), JsonValue::Null);
            auth.insert("session_token".to_owned(), JsonValue::Null);
            auth.insert("service_key_env".to_owned(), JsonValue::Null);
            auth.insert("credential".to_owned(), JsonValue::Null);
            if let Some(base_url) = non_empty(Some(draft.base_url.as_str())) {
                auth.insert(
                    "base_url".to_owned(),
                    JsonValue::String(base_url.to_owned()),
                );
            } else {
                auth.insert("base_url".to_owned(), JsonValue::Null);
            }
            if let Some(api_key_env) = non_empty(Some(draft.api_key_env.as_str())) {
                auth.insert(
                    "api_key_env".to_owned(),
                    JsonValue::String(api_key_env.to_owned()),
                );
            } else {
                auth.insert("api_key_env".to_owned(), JsonValue::Null);
            }
            if let Some(api_key) = non_empty(Some(draft.api_key.as_str())) {
                auth.insert("api_key".to_owned(), JsonValue::String(api_key.to_owned()));
            } else {
                auth.insert("api_key".to_owned(), JsonValue::Null);
            }
        }
        ProviderDraftAuthKind::Gitlab => {
            auth.insert("base_url".to_owned(), JsonValue::Null);
            if let Some(instance_url) = non_empty(Some(draft.instance_url.as_str())) {
                auth.insert(
                    "instance_url".to_owned(),
                    JsonValue::String(instance_url.to_owned()),
                );
            } else {
                auth.insert("instance_url".to_owned(), JsonValue::Null);
            }
            auth.insert("issuer".to_owned(), JsonValue::Null);
            auth.insert("region".to_owned(), JsonValue::Null);
            auth.insert("profile".to_owned(), JsonValue::Null);
            auth.insert("access_key_id".to_owned(), JsonValue::Null);
            auth.insert("secret_access_key".to_owned(), JsonValue::Null);
            auth.insert("session_token".to_owned(), JsonValue::Null);
            auth.insert("service_key_env".to_owned(), JsonValue::Null);
            auth.insert("credential".to_owned(), JsonValue::Null);
            if let Some(api_key_env) = non_empty(Some(draft.api_key_env.as_str())) {
                auth.insert(
                    "api_key_env".to_owned(),
                    JsonValue::String(api_key_env.to_owned()),
                );
            } else {
                auth.insert("api_key_env".to_owned(), JsonValue::Null);
            }
            if let Some(api_key) = non_empty(Some(draft.api_key.as_str())) {
                auth.insert("api_key".to_owned(), JsonValue::String(api_key.to_owned()));
            } else {
                auth.insert("api_key".to_owned(), JsonValue::Null);
            }
        }
        ProviderDraftAuthKind::Credential(None) => {
            return Err(anyhow!("credential_issuer is required before saving"));
        }
        ProviderDraftAuthKind::Credential(Some(_)) => {
            let issuer = parse_credential_issuer(draft.credential_issuer.as_str())?;
            auth.insert("api_key_env".to_owned(), JsonValue::Null);
            auth.insert("api_key".to_owned(), JsonValue::Null);
            auth.insert("region".to_owned(), JsonValue::Null);
            auth.insert("profile".to_owned(), JsonValue::Null);
            auth.insert("access_key_id".to_owned(), JsonValue::Null);
            auth.insert("secret_access_key".to_owned(), JsonValue::Null);
            auth.insert("session_token".to_owned(), JsonValue::Null);
            auth.insert(
                "issuer".to_owned(),
                JsonValue::String(credential_issuer_label(issuer).to_owned()),
            );
            if issuer == CredentialIssuer::Gitlab {
                if let Some(instance_url) = non_empty(Some(draft.instance_url.as_str())) {
                    auth.insert(
                        "instance_url".to_owned(),
                        JsonValue::String(instance_url.to_owned()),
                    );
                } else {
                    auth.insert("instance_url".to_owned(), JsonValue::Null);
                }
            } else {
                auth.insert("instance_url".to_owned(), JsonValue::Null);
            }
            if issuer.uses_http_endpoint() {
                if let Some(base_url) = non_empty(Some(draft.base_url.as_str())) {
                    auth.insert(
                        "base_url".to_owned(),
                        JsonValue::String(base_url.to_owned()),
                    );
                } else {
                    auth.insert("base_url".to_owned(), JsonValue::Null);
                }
            } else {
                auth.insert("base_url".to_owned(), JsonValue::Null);
            }
            if issuer.requires_service_key_env() {
                if let Some(service_key_env) = non_empty(Some(draft.service_key_env.as_str())) {
                    auth.insert(
                        "service_key_env".to_owned(),
                        JsonValue::String(service_key_env.to_owned()),
                    );
                } else {
                    auth.insert("service_key_env".to_owned(), JsonValue::Null);
                }
            } else {
                auth.insert("service_key_env".to_owned(), JsonValue::Null);
            }
            if let Some(credential) = inline_credential {
                auth.insert(
                    "credential".to_owned(),
                    serde_json::to_value(credential).map_err(api_error)?,
                );
            } else {
                auth.insert("credential".to_owned(), JsonValue::Null);
            }
        }
        ProviderDraftAuthKind::BedrockSigv4 => {
            auth.insert("api_key_env".to_owned(), JsonValue::Null);
            auth.insert("api_key".to_owned(), JsonValue::Null);
            auth.insert("instance_url".to_owned(), JsonValue::Null);
            auth.insert("credential".to_owned(), JsonValue::Null);
            auth.insert("issuer".to_owned(), JsonValue::Null);
            auth.insert("service_key_env".to_owned(), JsonValue::Null);
            if let Some(base_url) = non_empty(Some(draft.base_url.as_str())) {
                auth.insert(
                    "base_url".to_owned(),
                    JsonValue::String(base_url.to_owned()),
                );
            } else {
                auth.insert("base_url".to_owned(), JsonValue::Null);
            }
            if let Some(region) = non_empty(Some(draft.region.as_str())) {
                auth.insert("region".to_owned(), JsonValue::String(region.to_owned()));
            } else {
                auth.insert("region".to_owned(), JsonValue::Null);
            }
            if let Some(profile) = non_empty(Some(draft.profile.as_str())) {
                auth.insert("profile".to_owned(), JsonValue::String(profile.to_owned()));
            } else {
                auth.insert("profile".to_owned(), JsonValue::Null);
            }
            if let Some(access_key_id) = non_empty(Some(draft.access_key_id.as_str())) {
                auth.insert(
                    "access_key_id".to_owned(),
                    JsonValue::String(access_key_id.to_owned()),
                );
            } else {
                auth.insert("access_key_id".to_owned(), JsonValue::Null);
            }
            if let Some(secret_access_key) = non_empty(Some(draft.secret_access_key.as_str())) {
                auth.insert(
                    "secret_access_key".to_owned(),
                    JsonValue::String(secret_access_key.to_owned()),
                );
            } else {
                auth.insert("secret_access_key".to_owned(), JsonValue::Null);
            }
            if let Some(session_token) = non_empty(Some(draft.session_token.as_str())) {
                auth.insert(
                    "session_token".to_owned(),
                    JsonValue::String(session_token.to_owned()),
                );
            } else {
                auth.insert("session_token".to_owned(), JsonValue::Null);
            }
        }
    }
    Ok(auth)
}

fn provider_model_settings_path(provider_id: &str, adapter_id: &str, model_id: &str) -> String {
    format!(
        "providers.{}.adapters.{}.models.{}",
        quoted_settings_segment(provider_id),
        quoted_settings_segment(adapter_id),
        quoted_settings_segment(model_id),
    )
}

fn provider_settings_path(provider_id: &str) -> String {
    format!("providers.{}", quoted_settings_segment(provider_id))
}

fn provider_model_selection_contains(
    selected_model_keys: &std::collections::BTreeSet<String>,
    adapter_id: &str,
    model_id: &str,
) -> bool {
    selected_model_keys.contains(format!("{adapter_id}\u{1f}{model_id}").as_str())
}

fn resolve_provider_defaults_from_value(
    adapters: &JsonMap<String, JsonValue>,
    requested_default_adapter: Option<&str>,
    requested_default_model: Option<&str>,
) -> Result<(String, String)> {
    if let (Some(default_adapter), Some(default_model)) =
        (requested_default_adapter, requested_default_model)
        && provider_value_contains_model(adapters, default_adapter, default_model)
    {
        return Ok((default_adapter.to_owned(), default_model.to_owned()));
    }

    let mut adapter_ids = adapters.keys().cloned().collect::<Vec<_>>();
    adapter_ids.sort();
    for adapter_id in adapter_ids {
        let Some(adapter_value) = adapters.get(adapter_id.as_str()) else {
            continue;
        };
        let Some(model_ids) = adapter_value
            .get("models")
            .and_then(JsonValue::as_object)
            .map(|models| {
                let mut ids = models.keys().cloned().collect::<Vec<_>>();
                ids.sort();
                ids
            })
        else {
            continue;
        };
        if let Some(model_id) = model_ids.into_iter().next() {
            return Ok((adapter_id, model_id));
        }
    }

    Err(anyhow!(
        "select at least one model before saving the provider"
    ))
}

fn provider_value_contains_model(
    adapters: &JsonMap<String, JsonValue>,
    adapter_id: &str,
    model_id: &str,
) -> bool {
    adapters
        .get(adapter_id)
        .and_then(|adapter| adapter.get("models"))
        .and_then(JsonValue::as_object)
        .is_some_and(|models| models.contains_key(model_id))
}

fn quoted_settings_segment(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn credential_issuer_label(issuer: CredentialIssuer) -> &'static str {
    match issuer {
        CredentialIssuer::OpenaiChatgpt => "openai_chatgpt",
        CredentialIssuer::GithubCopilot => "github_copilot",
        CredentialIssuer::Gitlab => "gitlab",
        CredentialIssuer::GoogleAdc => "google_adc",
        CredentialIssuer::SapAiCore => "sap_ai_core",
        CredentialIssuer::AtomGit => "atomgit",
    }
}

fn parse_credential_issuer(value: &str) -> Result<CredentialIssuer> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai_chatgpt" => Ok(CredentialIssuer::OpenaiChatgpt),
        "github_copilot" => Ok(CredentialIssuer::GithubCopilot),
        "gitlab" => Ok(CredentialIssuer::Gitlab),
        "google_adc" => Ok(CredentialIssuer::GoogleAdc),
        "sap_ai_core" => Ok(CredentialIssuer::SapAiCore),
        "atomgit" | "atom_git" => Ok(CredentialIssuer::AtomGit),
        _ => Err(anyhow!(
            "unsupported credential issuer `{}`; expected openai_chatgpt, github_copilot, gitlab, google_adc, sap_ai_core, or atomgit",
            value.trim()
        )),
    }
}

fn summarize_named_mode(display_name: Option<&str>, description: Option<&str>) -> String {
    match (
        display_name
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        description.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(display_name), Some(description)) => format!("{display_name} · {description}"),
        (Some(display_name), None) => display_name.to_owned(),
        (None, Some(description)) => description.to_owned(),
        (None, None) => "configured mode".to_owned(),
    }
}

fn parse_aws_profile_names(text: &str) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if !line.starts_with('[') || !line.ends_with(']') {
            continue;
        }
        let section = line.trim_start_matches('[').trim_end_matches(']').trim();
        if section.eq_ignore_ascii_case("default") {
            names.insert("default".to_owned());
            continue;
        }
        if let Some(profile) = section.strip_prefix("profile ") {
            let profile = profile.trim();
            if !profile.is_empty() {
                names.insert(profile.to_owned());
            }
            continue;
        }
        if !section.contains(' ') && !section.contains('.') {
            names.insert(section.to_owned());
        }
    }
    names.into_iter().collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn provider_studio_test_draft(
        default_adapter: &str,
        default_model: &str,
        api_key: &str,
    ) -> ProviderConfigDraft {
        ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "oc".to_string(),
            auth_kind: ProviderDraftAuthKind::Api,
            base_url: "https://opencode.ai/zen".to_string(),
            instance_url: String::new(),
            api_key_env: String::new(),
            api_key: api_key.to_string(),
            credential_issuer: "openai_chatgpt".to_string(),
            region: String::new(),
            profile: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            session_token: String::new(),
            service_key_env: String::new(),
            credential_drafts: ProviderCredentialDraftBundle::default(),
            default_adapter: default_adapter.to_string(),
            default_model: default_model.to_string(),
        }
    }

    fn provider_studio_test_adapter_models(
        adapter_id: &str,
        resolved_base_url: &str,
        model_ids: &[&str],
    ) -> ProviderAdapterModelsResource {
        ProviderAdapterModelsResource {
            adapter_id: adapter_id.to_string(),
            enabled: true,
            resolved_base_url: Some(resolved_base_url.to_string()),
            models: model_ids
                .iter()
                .map(|model_id| ProviderModel::new(adapter_id, *model_id))
                .collect(),
            error: None,
        }
    }

    fn provider_studio_test_catalog_entry(
        model_id: &str,
        capabilities: agena::provider::ModelCapabilityPatch,
    ) -> ModelCatalogEntryResource {
        ModelCatalogEntryResource {
            model_id: model_id.to_string(),
            kind: ModelCatalogEntryKind::Official,
            source: ModelCatalogSourceKind::Generated,
            source_label: None,
            has_local_override: false,
            display_name: None,
            origin: None,
            lifecycle: None,
            context_window_tokens: None,
            max_input_tokens: None,
            max_output_tokens: None,
            description: None,
            knowledge_cutoff: None,
            release_date: None,
            last_updated: None,
            open_weights: None,
            default_thinking_mode: None,
            supports_parallel_tool_calls: None,
            supports_verbosity: None,
            default_verbosity: None,
            default_temperature: None,
            default_top_p: None,
            default_top_k: None,
            assistant_reasoning_interleaved: None,
            assistant_reasoning_field: None,
            output_modalities: Vec::new(),
            pricing: None,
            thinking_modes: std::collections::BTreeMap::new(),
            speed_modes: std::collections::BTreeMap::new(),
            capabilities,
        }
    }

    fn build_provider_save_patch(
        draft: &ProviderConfigDraft,
        adapter_model_lists: &[ProviderAdapterModelsResource],
        selected: &std::collections::BTreeSet<String>,
        catalog_entries: &[ModelCatalogEntryResource],
    ) -> JsonValue {
        let mut adapters = JsonMap::new();

        for adapter_models in adapter_model_lists {
            if adapter_models.error.is_some()
                || !selected.contains(adapter_models.adapter_id.as_str())
            {
                continue;
            }
            let configured_models = adapter_models
                .models
                .iter()
                .map(|model| {
                    (
                        model.id.to_string(),
                        provider_model_json_for_model_id(
                            &catalog_entries,
                            model.id.as_str(),
                            Some(model),
                        ),
                    )
                })
                .collect::<JsonMap<_, _>>();
            adapters.insert(
                adapter_models.adapter_id.clone(),
                json!({
                    "enabled": true,
                    "models": configured_models,
                }),
            );
        }

        let default_model_value =
            provider_model_json_for_model_id(catalog_entries, draft.default_model.as_str(), None);
        adapters
            .entry(draft.default_adapter.clone())
            .or_insert_with(|| json!({ "enabled": true }));
        ensure_provider_model_entry(
            adapters
                .get_mut(draft.default_adapter.as_str())
                .expect("default adapter patch should exist"),
            draft.default_model.as_str(),
            default_model_value,
        )
        .expect("default model patch should be inserted");

        build_provider_patch_value(
            &draft,
            draft.default_adapter.as_str(),
            draft.default_model.as_str(),
            JsonValue::Object(adapters),
            true,
        )
        .expect("provider patch should build")
    }

    fn build_api_provider_save_patch() -> JsonValue {
        let draft = provider_studio_test_draft("gemini", "deepseek-v4-flash-free", "test-token");
        let adapter_model_lists = vec![
            provider_studio_test_adapter_models(
                "openai",
                "https://opencode.ai/zen/v1",
                &["deepseek-v4-flash-free"],
            ),
            provider_studio_test_adapter_models(
                "anthropic",
                "https://opencode.ai/zen/v1",
                &["deepseek-v4-flash-free"],
            ),
            ProviderAdapterModelsResource {
                adapter_id: "gemini".to_string(),
                enabled: true,
                resolved_base_url: Some("https://opencode.ai/zen/v1beta".to_string()),
                models: Vec::new(),
                error: Some("google api request failed".to_string()),
            },
        ];
        let selected =
            std::collections::BTreeSet::from(["openai".to_string(), "anthropic".to_string()]);
        build_provider_save_patch(&draft, &adapter_model_lists, &selected, &[])
    }

    #[test]
    fn provider_patch_accepts_api_provider_with_unselected_default_adapter() {
        let provider_patch = build_api_provider_save_patch();
        let config = NamedTempFile::new().expect("temp config should exist");
        patch_file_settings(
            config.path(),
            ConfigSettingsPatchInput {
                path: Some("providers".to_string()),
                changes: json!({
                    "oc": provider_patch,
                }),
                dry_run: false,
                validate: true,
                reload: false,
            },
        )
        .expect("provider patch should validate and write");
    }

    #[test]
    fn provider_set_save_replaces_existing_provider_table() {
        let provider_patch = build_api_provider_save_patch();
        let config = NamedTempFile::new().expect("temp config should exist");
        fs::write(
            config.path(),
            r#"
[providers.oc]
default_adapter = "gitlab"
default_model = "duo-chat-gpt-5-2"

[providers.oc.auth]
mode = "gitlab_api"
api_key_env = "GITLAB_TOKEN"

[providers.oc.adapters.gitlab]
enabled = true
"#,
        )
        .expect("seed config should write");

        set_file_setting(
            config.path(),
            ConfigSettingsSetInput {
                path: "providers.\"oc\"".to_string(),
                value: provider_patch,
                dry_run: false,
                validate: true,
                reload: false,
            },
        )
        .expect("provider table replacement should validate and write");

        let text = fs::read_to_string(config.path()).expect("config should be readable");
        assert!(!text.contains("[providers.oc.adapters.gitlab]"));
        assert!(text.contains("[providers.oc.adapters.openai]"));
        assert!(text.contains("[providers.oc.adapters.anthropic]"));
        assert!(text.contains("[providers.oc.adapters.gemini]"));
    }

    #[test]
    fn build_provider_auth_patch_value_preserves_inline_gitlab_oauth_credential() {
        let mut draft = provider_studio_test_draft("openai", "duo-chat-gpt-5-2", "");
        draft.provider_id = "gitlab-oauth".to_string();
        draft.auth_kind = ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab));
        draft.credential_issuer = "gitlab".to_string();
        draft.instance_url = "https://gitlab.example.com".to_string();
        draft.credential_drafts.gitlab.tokens.refresh_token = "refresh-token".to_string();
        draft.credential_drafts.gitlab.tokens.access_token = "access-token".to_string();
        draft.credential_drafts.gitlab.tokens.expires_at_ms = "123".to_string();
        draft.normalize_shape();

        let auth = build_provider_auth_patch_value(&draft).expect("auth patch should build");
        assert_eq!(auth.get("mode"), Some(&json!("credential")));
        assert_eq!(auth.get("issuer"), Some(&json!("gitlab")));
        assert_eq!(
            auth.get("instance_url"),
            Some(&json!("https://gitlab.example.com"))
        );

        let credential = auth
            .get("credential")
            .and_then(JsonValue::as_object)
            .expect("inline credential should be present");
        assert_eq!(credential.get("type"), Some(&json!("oauth")));
        assert_eq!(credential.get("issuer"), Some(&json!("gitlab")));
        assert_eq!(credential.get("refresh"), Some(&json!("refresh-token")));
        assert_eq!(credential.get("access"), Some(&json!("access-token")));
        assert_eq!(credential.get("expires_at_ms"), Some(&json!(123)));
    }

    #[test]
    fn provider_patch_accepts_opencode_zen_public_with_all_openai_and_anthropic_models() {
        use agena::provider::{
            FeatureCapabilityPatch, FeatureCapabilityPatchBody, ModelCapabilityFeature,
            ModelCapabilityPatch,
        };

        let draft = provider_studio_test_draft("openai", "glm-5", "public");
        let model_ids = [
            "gpt-5.5",
            "glm-5",
            "claude-opus-4-7",
            "qwen3.6-plus",
            "minimax-m2.5",
        ];
        let adapter_model_lists = vec![
            provider_studio_test_adapter_models("openai", "https://opencode.ai/zen/v1", &model_ids),
            provider_studio_test_adapter_models(
                "anthropic",
                "https://opencode.ai/zen/v1",
                &model_ids,
            ),
        ];
        let selected =
            std::collections::BTreeSet::from(["openai".to_string(), "anthropic".to_string()]);
        let catalog_entries = vec![
            provider_studio_test_catalog_entry(
                "glm-5",
                ModelCapabilityPatch {
                    features: Some(FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                        supported: vec![
                            ModelCapabilityFeature::StructuredOutput,
                            ModelCapabilityFeature::Reasoning,
                        ],
                        unsupported: vec![ModelCapabilityFeature::StructuredOutput],
                    })),
                    ..ModelCapabilityPatch::default()
                },
            ),
            provider_studio_test_catalog_entry("gpt-5.5", ModelCapabilityPatch::default()),
            provider_studio_test_catalog_entry("claude-opus-4-7", ModelCapabilityPatch::default()),
            provider_studio_test_catalog_entry("qwen3.6-plus", ModelCapabilityPatch::default()),
            provider_studio_test_catalog_entry("minimax-m2.5", ModelCapabilityPatch::default()),
        ];
        let provider_patch =
            build_provider_save_patch(&draft, &adapter_model_lists, &selected, &catalog_entries);
        let config = NamedTempFile::new().expect("temp config should exist");

        set_file_setting(
            config.path(),
            ConfigSettingsSetInput {
                path: "providers.\"oc\"".to_string(),
                value: provider_patch,
                dry_run: false,
                validate: true,
                reload: false,
            },
        )
        .expect("opencode zen provider patch should validate and write");

        let text = fs::read_to_string(config.path()).expect("config should be readable");
        assert!(text.contains("base_url = \"https://opencode.ai/zen\""));
        assert!(text.contains("api_key = \"public\""));
        for path in [
            "providers.\"oc\".adapters.openai.models.\"glm-5\"",
            "providers.\"oc\".adapters.anthropic.models.\"glm-5\"",
            "providers.\"oc\".adapters.openai.models.\"claude-opus-4-7\"",
            "providers.\"oc\".adapters.anthropic.models.\"claude-opus-4-7\"",
            "providers.\"oc\".adapters.openai.models.\"qwen3.6-plus\"",
            "providers.\"oc\".adapters.anthropic.models.\"qwen3.6-plus\"",
        ] {
            let value = read_file_setting(
                config.path(),
                ConfigSettingsGetInput {
                    path: Some(path.to_string()),
                    source: agena::config::ConfigSettingsSource::File,
                },
            )
            .expect("saved model entry should be readable")
            .value;
            assert!(value.is_object(), "{path} should exist as an object");
        }
    }
}
