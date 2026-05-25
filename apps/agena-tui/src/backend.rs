use std::{
    collections::HashSet,
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, OnceLock},
};

use agena::event::{EventFilter, Scope, bus::SubscriptionItem};
use agena::permission::PermissionScope;
use agena::{
    agents::AgentDescriptor,
    config::{
        ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsEditResponse,
        ConfigSettingsGetInput, ConfigSettingsPatchInput, ConfigSettingsPathInput,
        ConfigSettingsSetInput, ProcessEnvironment, ProviderAdapterOverlay, ProviderAuthConfig,
        ProviderAuthMode, ProviderAuthOverlay, ProviderModelOverlay, ProviderNativeToolRoute,
        ProviderNativeToolsConfig, ProviderOverlay, delete_file_setting,
        draft_atomgit_provider_adapter_models_target, draft_gitlab_provider_adapter_models_target,
        draft_provider_adapter_models_target, list_provider_adapter_models_with_config,
        patch_file_settings, provider_model_overlay_from_catalog_definition, read_file_setting,
        saved_provider_adapter_models_target, set_file_setting,
    },
    event::{DomainEvent, EventKind},
    memory::MemoryStore,
    message::{
        AttachmentItem, AttachmentKind, AttachmentSource, EnterWorktreeToolInput,
        ExitWorktreeToolInput, PartContent, ToolInvocation, UserInputReply,
    },
    model::ModelRef,
    model_catalog::{CatalogModelDefinition, catalog_definition_from_model},
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

fn provider_native_tools_summary_resource(
    provider: &agena::config::ResolvedProviderConfig,
) -> ProviderNativeToolsSummaryResource {
    let (enabled, default_bindings) = provider
        .defaults
        .adapter
        .as_ref()
        .zip(provider.defaults.model.as_ref())
        .and_then(|(adapter_id, model_id)| {
            provider
                .models
                .get(format!("{adapter_id}/{model_id}").as_str())
        })
        .map(|model| (model.native_tools.enabled, model.native_tool_bindings()))
        .unwrap_or((false, Vec::new()));
    ProviderNativeToolsSummaryResource {
        enabled,
        model_count: provider
            .models
            .values()
            .filter(|model| model.native_tools.enabled)
            .count(),
        bindings: default_bindings
            .into_iter()
            .map(|binding| ProviderNativeToolBindingResource {
                tool: binding.tool.config_key().to_owned(),
                route: serde_json::to_string(&binding.route)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned(),
            })
            .collect(),
    }
}

fn provider_native_tools_config_for_preset(
    preset: ProviderNativeToolsPreset,
    custom: &ProviderNativeToolsConfig,
) -> ProviderNativeToolsConfig {
    match preset {
        ProviderNativeToolsPreset::Disabled => ProviderNativeToolsConfig::default(),
        ProviderNativeToolsPreset::OpenAiHostedDefaults => ProviderNativeToolsConfig {
            enabled: true,
            routes: agena::config::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                file_search: Some(ProviderNativeToolRoute::ProviderHosted),
                code_execution: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::AnthropicHostedDefaults => ProviderNativeToolsConfig {
            enabled: true,
            routes: agena::config::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::GeminiHostedDefaults => ProviderNativeToolsConfig {
            enabled: true,
            routes: agena::config::ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                code_execution: Some(ProviderNativeToolRoute::ProviderHosted),
                url_context: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            ..Default::default()
        },
        ProviderNativeToolsPreset::Custom => custom.clone(),
    }
}

fn provider_native_tools_preset_from_config(
    config: &ProviderNativeToolsConfig,
) -> ProviderNativeToolsPreset {
    if *config
        == provider_native_tools_config_for_preset(
            ProviderNativeToolsPreset::OpenAiHostedDefaults,
            &ProviderNativeToolsConfig::default(),
        )
    {
        ProviderNativeToolsPreset::OpenAiHostedDefaults
    } else if *config
        == provider_native_tools_config_for_preset(
            ProviderNativeToolsPreset::AnthropicHostedDefaults,
            &ProviderNativeToolsConfig::default(),
        )
    {
        ProviderNativeToolsPreset::AnthropicHostedDefaults
    } else if *config
        == provider_native_tools_config_for_preset(
            ProviderNativeToolsPreset::GeminiHostedDefaults,
            &ProviderNativeToolsConfig::default(),
        )
    {
        ProviderNativeToolsPreset::GeminiHostedDefaults
    } else if config.is_empty() {
        ProviderNativeToolsPreset::Disabled
    } else {
        ProviderNativeToolsPreset::Custom
    }
}

fn provider_draft_base_url_host(value: &str) -> Option<String> {
    let parsed = url::Url::parse(value.trim()).ok()?;
    parsed.host_str().map(|host| host.to_ascii_lowercase())
}

use agena_api::{
    commands::{
        Command as ApiCommand, CommandResult, CompactSessionParams, ContinueRunParams,
        CreateSessionParams, ReplacePermissionRuleParams, ReplyPermissionParams,
        ReplyUserInputParams, RewindSessionParams, SubmitMessageParams, UpdateSessionParams,
        UpsertPermissionRuleParams,
    },
    pagination::PaginatedResponse,
    queries::{
        GetSessionParams, ListMessagesParams, ListPermissionRulesParams, ListSessionsParams, Query,
        QueryResult,
    },
    resource::{
        MessageResource, PartLoadMode, PermissionReply, PermissionRuleResource,
        ProviderAdapterModelsResource, ProviderAdapterModelsResponse,
        ProviderAdapterSummaryResource, ProviderDefaultsResource,
        ProviderNativeToolBindingResource, ProviderNativeToolsSummaryResource,
        ProviderSummaryResource, RunOptions, SessionExecutionResource, SessionResource,
        WorkspaceResource,
    },
};
use agena_api_server::{
    dispatch,
    local_api::{
        ModelCatalogEntryResource, ModelCatalogListResponse,
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

#[derive(Debug, Clone)]
pub struct SessionPermissionStudioState {
    pub session_title: String,
    pub permission: agena::agent::PermissionConfig,
    pub effective_permission: agena::agent::PermissionConfig,
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
    pub detail_key: &'static str,
    pub requires_base_url: bool,
    pub supports_draft_model_listing: bool,
}

const NONE_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "ollama",
    detail_key: "provider-adapter-rule-none-ollama-detail",
    requires_base_url: false,
    supports_draft_model_listing: false,
}];

const API_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai",
        detail_key: "provider-adapter-rule-api-openai-detail",
        requires_base_url: true,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail_key: "provider-adapter-rule-api-anthropic-detail",
        requires_base_url: true,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "gemini",
        detail_key: "provider-adapter-rule-api-gemini-detail",
        requires_base_url: true,
        supports_draft_model_listing: true,
    },
];

const GITLAB_AUTH_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai",
        detail_key: "provider-adapter-rule-gitlab-auth-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail_key: "provider-adapter-rule-gitlab-auth-anthropic-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
];

const OPENAI_CHATGPT_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "openai",
    detail_key: "provider-adapter-rule-openai-chatgpt-openai-detail",
    requires_base_url: false,
    supports_draft_model_listing: false,
}];

const GITHUB_COPILOT_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai",
        detail_key: "provider-adapter-rule-github-copilot-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: false,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail_key: "provider-adapter-rule-github-copilot-anthropic-detail",
        requires_base_url: false,
        supports_draft_model_listing: false,
    },
];

const GITLAB_CREDENTIAL_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai",
        detail_key: "provider-adapter-rule-gitlab-credential-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: false,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail_key: "provider-adapter-rule-gitlab-credential-anthropic-detail",
        requires_base_url: false,
        supports_draft_model_listing: false,
    },
];

const GOOGLE_ADC_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "openai",
    detail_key: "provider-adapter-rule-google-adc-openai-detail",
    requires_base_url: true,
    supports_draft_model_listing: false,
}];

const SAP_AI_CORE_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "openai",
    detail_key: "provider-adapter-rule-sap-ai-core-openai-detail",
    requires_base_url: true,
    supports_draft_model_listing: false,
}];

const ATOMGIT_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "openai",
    detail_key: "provider-adapter-rule-atomgit-openai-detail",
    requires_base_url: false,
    supports_draft_model_listing: true,
}];

const BEDROCK_SIGV4_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "amazon_bedrock",
    detail_key: "provider-adapter-rule-bedrock-sigv4-amazon-bedrock-detail",
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

    fn active_tokens(&self, issuer: Option<CredentialIssuer>) -> Option<&ProviderOAuthTokensDraft> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => Some(&self.openai_chatgpt.tokens),
            Some(CredentialIssuer::GithubCopilot) => Some(&self.github_copilot.tokens),
            Some(CredentialIssuer::Gitlab) => Some(&self.gitlab.tokens),
            Some(CredentialIssuer::AtomGit) => Some(&self.atomgit.tokens),
            Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => None,
        }
    }

    fn active_tokens_mut(
        &mut self,
        issuer: Option<CredentialIssuer>,
    ) -> Option<&mut ProviderOAuthTokensDraft> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => Some(&mut self.openai_chatgpt.tokens),
            Some(CredentialIssuer::GithubCopilot) => Some(&mut self.github_copilot.tokens),
            Some(CredentialIssuer::Gitlab) => Some(&mut self.gitlab.tokens),
            Some(CredentialIssuer::AtomGit) => Some(&mut self.atomgit.tokens),
            Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => None,
        }
    }

    fn redirect_uri(&self, issuer: Option<CredentialIssuer>) -> Option<&str> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => {
                Some(self.openai_chatgpt.redirect_uri.as_str())
            }
            Some(CredentialIssuer::Gitlab) => Some(self.gitlab.redirect_uri.as_str()),
            _ => None,
        }
    }

    fn callback_url(&self, issuer: Option<CredentialIssuer>) -> Option<&str> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => {
                Some(self.openai_chatgpt.callback_url.as_str())
            }
            Some(CredentialIssuer::Gitlab) => Some(self.gitlab.callback_url.as_str()),
            _ => None,
        }
    }

    fn account_id(&self, issuer: Option<CredentialIssuer>) -> Option<&str> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => Some(self.openai_chatgpt.account_id.as_str()),
            Some(CredentialIssuer::AtomGit) => Some(self.atomgit.account_id.as_str()),
            _ => None,
        }
    }

    fn set_redirect_uri(&mut self, issuer: Option<CredentialIssuer>, value: String) {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => self.openai_chatgpt.redirect_uri = value,
            Some(CredentialIssuer::Gitlab) => self.gitlab.redirect_uri = value,
            _ => {}
        }
    }

    fn set_callback_url(&mut self, issuer: Option<CredentialIssuer>, value: String) {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => self.openai_chatgpt.callback_url = value,
            Some(CredentialIssuer::Gitlab) => self.gitlab.callback_url = value,
            _ => {}
        }
    }

    fn set_account_id(&mut self, issuer: Option<CredentialIssuer>, value: String) {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => self.openai_chatgpt.account_id = value,
            Some(CredentialIssuer::AtomGit) => self.atomgit.account_id = value,
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProviderDraftAuthMessage {
    OpenaiBrowserStarted,
    CopilotDeviceStarted { user_code: String },
    GitlabBrowserStarted,
    AtomGitBrowserStarted,
    OpenaiCredentialCaptured,
    CopilotPending,
    CopilotCredentialCaptured,
    GitlabCredentialCaptured,
    AtomGitPending,
    AtomGitCredentialCaptured,
}

#[derive(Debug, Clone)]
pub enum ProviderDraftAuthField {
    RedirectUri,
    InstanceUrl,
    CallbackUrl,
}

#[derive(Debug, Clone)]
pub enum ProviderDraftAuthError {
    UnsupportedInteractiveLogin,
    StartBrowserAuthFirst,
    StartDeviceAuthFirst,
    RequiredField(ProviderDraftAuthField),
    Other(String),
}

impl ProviderDraftAuthError {
    fn other(error: impl ToString) -> Self {
        Self::Other(error.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct ProviderDraftAuthActionResult {
    pub draft: ProviderConfigDraft,
    pub message: ProviderDraftAuthMessage,
    pub clipboard_text: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ProviderStudioSaveResult {
    ProviderDraftSaved {
        provider_id: String,
        default_adapter: String,
        default_model: String,
    },
    AdapterMatchesSaved {
        provider_id: String,
        adapter_id: String,
        listed_model_count: usize,
        matched_model_count: usize,
    },
    ModelSaved {
        provider_id: String,
        adapter_id: String,
        model_id: String,
    },
    ConfiguredModelSaved {
        provider_id: String,
        adapter_id: String,
        model_id: String,
    },
}

#[derive(Debug, Clone)]
pub enum ProviderStudioSaveField {
    ProviderId,
    DefaultAdapter,
    AdapterId,
    ModelId,
    AuthMode,
    CredentialIssuer,
}

#[derive(Debug, Clone)]
pub enum ProviderStudioSaveValidationError {
    FieldRequired(ProviderStudioSaveField),
    UnsupportedDefaultAdapter {
        auth_kind: ProviderDraftAuthKind,
        adapter: String,
        supported: String,
    },
    UnsupportedAdapters {
        auth_kind: ProviderDraftAuthKind,
        adapters: Vec<String>,
        supported: String,
    },
    ApiBaseUrlRequired,
    GitlabApiKeyOrEnvRequired,
    CredentialBaseUrlRequired {
        issuer: CredentialIssuer,
    },
    CredentialServiceKeyEnvRequired {
        issuer: CredentialIssuer,
    },
    BedrockKeyPairRequired,
    SelectAtLeastOneModel,
}

#[derive(Debug, Clone)]
pub enum ProviderStudioSaveError {
    Validation(ProviderStudioSaveValidationError),
    ExistingProviderSettingsMustBeObject,
    ProviderAdapterMustBeObject { adapter_id: String },
    ProviderModelConfigMustBeObject,
    ConfiguredProviderAdapterSettingsMustBeObject,
    ConfiguredProviderAdapterModelsMustBeObject,
    Other(String),
}

impl ProviderStudioSaveError {
    fn other(error: impl ToString) -> Self {
        Self::Other(error.to_string())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderDraftAuthDetails {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderNativeToolsPreset {
    Disabled,
    OpenAiHostedDefaults,
    AnthropicHostedDefaults,
    GeminiHostedDefaults,
    Custom,
}

impl ProviderNativeToolsPreset {
    pub fn token(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::OpenAiHostedDefaults => "openai_hosted_defaults",
            Self::AnthropicHostedDefaults => "anthropic_hosted_defaults",
            Self::GeminiHostedDefaults => "gemini_hosted_defaults",
            Self::Custom => "custom",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "disabled" => Some(Self::Disabled),
            "openai_hosted_defaults" => Some(Self::OpenAiHostedDefaults),
            "anthropic_hosted_defaults" => Some(Self::AnthropicHostedDefaults),
            "gemini_hosted_defaults" => Some(Self::GeminiHostedDefaults),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderConfigDraft {
    pub source_provider_id: Option<String>,
    pub provider_id: String,
    pub auth_kind: ProviderDraftAuthKind,
    pub auth: ProviderDraftAuthDetails,
    pub credential_drafts: ProviderCredentialDraftBundle,
    pub default_adapter: String,
    pub default_model: String,
    pub native_tools_preset: ProviderNativeToolsPreset,
    pub native_tools_custom: ProviderNativeToolsConfig,
    pub native_tools_touched: bool,
}

impl ProviderConfigDraft {
    pub fn new_empty() -> Self {
        Self {
            source_provider_id: None,
            provider_id: String::new(),
            auth_kind: ProviderDraftAuthKind::Unset,
            auth: ProviderDraftAuthDetails::default(),
            credential_drafts: ProviderCredentialDraftBundle::default(),
            default_adapter: String::new(),
            default_model: String::new(),
            native_tools_preset: ProviderNativeToolsPreset::Disabled,
            native_tools_custom: ProviderNativeToolsConfig::default(),
            native_tools_touched: false,
        }
    }

    pub fn normalize_shape(&mut self) {
        self.credential_drafts.normalize_shape();
        self.auth.credential_issuer = self
            .auth_kind
            .credential_issuer()
            .map(credential_issuer_label)
            .unwrap_or_default()
            .to_owned();

        match self.auth_kind {
            ProviderDraftAuthKind::Unset => {
                self.auth.base_url.clear();
                self.auth.api_key_env.clear();
                self.auth.api_key.clear();
                self.auth.region.clear();
                self.auth.profile.clear();
                self.auth.access_key_id.clear();
                self.auth.secret_access_key.clear();
                self.auth.session_token.clear();
                self.auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::None => {
                self.auth.base_url.clear();
                self.auth.api_key_env.clear();
                self.auth.api_key.clear();
                self.auth.region.clear();
                self.auth.profile.clear();
                self.auth.access_key_id.clear();
                self.auth.secret_access_key.clear();
                self.auth.session_token.clear();
                self.auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::Api => {
                self.auth.region.clear();
                self.auth.profile.clear();
                self.auth.access_key_id.clear();
                self.auth.secret_access_key.clear();
                self.auth.session_token.clear();
                self.auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::Gitlab => {
                self.auth.base_url.clear();
                self.auth.region.clear();
                self.auth.profile.clear();
                self.auth.access_key_id.clear();
                self.auth.secret_access_key.clear();
                self.auth.session_token.clear();
                self.auth.service_key_env.clear();
                if self.auth.instance_url.trim().is_empty() {
                    self.auth.instance_url = DEFAULT_GITLAB_INSTANCE_URL.to_owned();
                }
            }
            ProviderDraftAuthKind::Credential(None) => {
                self.auth.base_url.clear();
                self.auth.api_key_env.clear();
                self.auth.api_key.clear();
                self.auth.region.clear();
                self.auth.profile.clear();
                self.auth.access_key_id.clear();
                self.auth.secret_access_key.clear();
                self.auth.session_token.clear();
                self.auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::Credential(Some(issuer)) => {
                self.auth.api_key_env.clear();
                self.auth.api_key.clear();
                self.auth.region.clear();
                self.auth.profile.clear();
                self.auth.access_key_id.clear();
                self.auth.secret_access_key.clear();
                self.auth.session_token.clear();
                if !issuer.uses_http_endpoint() {
                    self.auth.base_url.clear();
                }
                if issuer == CredentialIssuer::Gitlab && self.auth.instance_url.trim().is_empty() {
                    self.auth.instance_url = DEFAULT_GITLAB_INSTANCE_URL.to_owned();
                }
                if issuer.requires_service_key_env() {
                    if self.auth.service_key_env.trim().is_empty() {
                        self.auth.service_key_env = "AICORE_SERVICE_KEY".to_owned();
                    }
                } else {
                    self.auth.service_key_env.clear();
                }
            }
            ProviderDraftAuthKind::BedrockSigv4 => {
                self.auth.api_key_env.clear();
                self.auth.api_key.clear();
                self.auth.service_key_env.clear();
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
        self.sync_native_tools_suggestion();
    }

    pub fn from_resolved(
        provider_id: &str,
        provider: &agena::config::ResolvedProviderConfig,
    ) -> Self {
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
            ProviderAuthConfig::Api(api) => (
                ProviderDraftAuthKind::Api,
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
            ),
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

        let default_native_tools = provider
            .defaults
            .adapter
            .as_ref()
            .zip(provider.defaults.model.as_ref())
            .and_then(|(adapter_id, model_id)| {
                provider
                    .models
                    .get(format!("{adapter_id}/{model_id}").as_str())
                    .map(|model| model.native_tools.clone())
            })
            .unwrap_or_default();

        let mut draft = Self {
            source_provider_id: Some(provider_id.to_owned()),
            provider_id: provider_id.to_owned(),
            auth_kind,
            auth: ProviderDraftAuthDetails {
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
            },
            credential_drafts,
            default_adapter: provider.defaults.adapter.clone().unwrap_or_default(),
            default_model: provider.defaults.model.clone().unwrap_or_default(),
            native_tools_preset: provider_native_tools_preset_from_config(&default_native_tools),
            native_tools_custom: default_native_tools,
            native_tools_touched: true,
        };
        draft.normalize_shape();
        draft
    }

    fn to_provider_overlay_for_save(
        &self,
        default_adapter: &str,
        default_model: &str,
        adapters: std::collections::BTreeMap<String, ProviderAdapterOverlay>,
        include_defaults: bool,
    ) -> std::result::Result<ProviderOverlay, ProviderStudioSaveError> {
        Ok(ProviderOverlay {
            enabled: Some(true),
            defaults: include_defaults.then(|| agena::config::ProviderDefaultsOverlay {
                adapter: Some(default_adapter.to_owned()),
                model: Some(default_model.to_owned()),
                ..Default::default()
            }),
            auth: Some(self.to_auth_overlay_for_save()?),
            adapters,
        })
    }

    fn to_auth_overlay_for_save(
        &self,
    ) -> std::result::Result<ProviderAuthOverlay, ProviderStudioSaveError> {
        let credential = self
            .oauth_auth_data()
            .map_err(ProviderStudioSaveError::other)?;
        let mut overlay = ProviderAuthOverlay {
            mode: Some(self.to_provider_auth_mode_for_save()?),
            ..ProviderAuthOverlay::default()
        };

        match self.auth_kind {
            ProviderDraftAuthKind::Unset => {
                return Err(ProviderStudioSaveError::Validation(
                    ProviderStudioSaveValidationError::FieldRequired(
                        ProviderStudioSaveField::AuthMode,
                    ),
                ));
            }
            ProviderDraftAuthKind::None => {}
            ProviderDraftAuthKind::Api => {
                overlay.base_url = trimmed_owned(self.auth.base_url.as_str());
                overlay.api_key_env = trimmed_owned(self.auth.api_key_env.as_str());
                overlay.api_key = trimmed_owned(self.auth.api_key.as_str());
            }
            ProviderDraftAuthKind::Gitlab => {
                overlay.instance_url = trimmed_owned(self.auth.instance_url.as_str());
                overlay.api_key_env = trimmed_owned(self.auth.api_key_env.as_str());
                overlay.api_key = trimmed_owned(self.auth.api_key.as_str());
            }
            ProviderDraftAuthKind::Credential(None) => {
                return Err(ProviderStudioSaveError::Validation(
                    ProviderStudioSaveValidationError::FieldRequired(
                        ProviderStudioSaveField::CredentialIssuer,
                    ),
                ));
            }
            ProviderDraftAuthKind::Credential(Some(_)) => {
                let issuer = parse_credential_issuer(self.auth.credential_issuer.as_str())
                    .map_err(ProviderStudioSaveError::other)?;
                overlay.issuer = Some(issuer);
                if issuer == CredentialIssuer::Gitlab {
                    overlay.instance_url = trimmed_owned(self.auth.instance_url.as_str());
                }
                if issuer.uses_http_endpoint() {
                    overlay.base_url = trimmed_owned(self.auth.base_url.as_str());
                }
                if issuer.requires_service_key_env() {
                    overlay.service_key_env = trimmed_owned(self.auth.service_key_env.as_str());
                }
                overlay.credential = credential;
            }
            ProviderDraftAuthKind::BedrockSigv4 => {
                overlay.base_url = trimmed_owned(self.auth.base_url.as_str());
                overlay.region = trimmed_owned(self.auth.region.as_str());
                overlay.profile = trimmed_owned(self.auth.profile.as_str());
                overlay.access_key_id = trimmed_owned(self.auth.access_key_id.as_str());
                overlay.secret_access_key = trimmed_owned(self.auth.secret_access_key.as_str());
                overlay.session_token = trimmed_owned(self.auth.session_token.as_str());
            }
        }

        Ok(overlay)
    }

    fn to_provider_auth_mode_for_save(
        &self,
    ) -> std::result::Result<ProviderAuthMode, ProviderStudioSaveError> {
        match self.auth_kind {
            ProviderDraftAuthKind::Unset => Err(ProviderStudioSaveError::Validation(
                ProviderStudioSaveValidationError::FieldRequired(ProviderStudioSaveField::AuthMode),
            )),
            ProviderDraftAuthKind::None => Ok(ProviderAuthMode::None),
            ProviderDraftAuthKind::Api => Ok(ProviderAuthMode::Api),
            ProviderDraftAuthKind::Gitlab => Ok(ProviderAuthMode::Gitlab),
            ProviderDraftAuthKind::Credential(_) => Ok(ProviderAuthMode::Credential),
            ProviderDraftAuthKind::BedrockSigv4 => Ok(ProviderAuthMode::BedrockSigv4),
        }
    }

    fn apply_native_tools_to_model_overlay(
        &self,
        adapter_id: &str,
        model_id: &str,
        mut overlay: ProviderModelOverlay,
    ) -> ProviderModelOverlay {
        if self
            .default_model_route()
            .is_some_and(|route| route == provider_model_route_id(adapter_id, model_id))
        {
            overlay.native_tools = self.effective_native_tools_config();
        }
        overlay
    }

    pub fn suggested_native_tools_preset(&self) -> Option<ProviderNativeToolsPreset> {
        match self.auth_kind {
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
                if self.default_adapter.trim() == "openai" =>
            {
                Some(ProviderNativeToolsPreset::OpenAiHostedDefaults)
            }
            ProviderDraftAuthKind::Api => {
                let Some(host) = provider_draft_base_url_host(self.auth.base_url.as_str()) else {
                    return None;
                };
                match (host.as_str(), self.default_adapter.trim()) {
                    ("api.openai.com", "openai") => {
                        Some(ProviderNativeToolsPreset::OpenAiHostedDefaults)
                    }
                    ("api.anthropic.com" | "api-staging.anthropic.com", "anthropic") => {
                        Some(ProviderNativeToolsPreset::AnthropicHostedDefaults)
                    }
                    ("generativelanguage.googleapis.com", "gemini") => {
                        Some(ProviderNativeToolsPreset::GeminiHostedDefaults)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn available_native_tools_preset(&self) -> Option<ProviderNativeToolsPreset> {
        match self.default_adapter.trim() {
            "openai" => Some(ProviderNativeToolsPreset::OpenAiHostedDefaults),
            "anthropic" => Some(ProviderNativeToolsPreset::AnthropicHostedDefaults),
            "gemini" => Some(ProviderNativeToolsPreset::GeminiHostedDefaults),
            _ => None,
        }
    }

    pub fn supports_native_tools_preset(&self, preset: ProviderNativeToolsPreset) -> bool {
        match preset {
            ProviderNativeToolsPreset::Disabled | ProviderNativeToolsPreset::Custom => true,
            other => self.available_native_tools_preset() == Some(other),
        }
    }

    pub fn sync_native_tools_suggestion(&mut self) {
        if !self.supports_native_tools_preset(self.native_tools_preset) {
            self.native_tools_preset = ProviderNativeToolsPreset::Disabled;
            self.native_tools_touched = self.source_provider_id.is_some();
        }
        if self.native_tools_touched || self.source_provider_id.is_some() {
            return;
        }
        self.native_tools_preset = self
            .suggested_native_tools_preset()
            .unwrap_or(ProviderNativeToolsPreset::Disabled);
    }

    pub fn set_native_tools_preset(&mut self, preset: ProviderNativeToolsPreset) {
        self.native_tools_preset = preset;
        self.native_tools_touched = true;
    }

    pub fn effective_native_tools_config(&self) -> ProviderNativeToolsConfig {
        provider_native_tools_config_for_preset(self.native_tools_preset, &self.native_tools_custom)
    }

    fn default_model_route(&self) -> Option<String> {
        let adapter_id = optional_non_empty(self.default_adapter.as_str())?;
        let model_id = optional_non_empty(self.default_model.as_str())?;
        Some(provider_model_route_id(adapter_id, model_id))
    }

    fn oauth_auth_data(&self) -> Result<Option<AuthData>> {
        match self.auth_kind {
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
                let tokens = &self.credential_drafts.openai_chatgpt.tokens;
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
                        self.credential_drafts.openai_chatgpt.account_id.as_str(),
                    )
                    .map(ToOwned::to_owned),
                    enterprise_url: None,
                    user: None,
                }))
            }
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
                let tokens = &self.credential_drafts.github_copilot.tokens;
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
                        self.credential_drafts
                            .github_copilot
                            .enterprise_domain
                            .as_str(),
                    )
                    .map(ToOwned::to_owned),
                    user: None,
                }))
            }
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
                let tokens = &self.credential_drafts.gitlab.tokens;
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
                let tokens = &self.credential_drafts.atomgit.tokens;
                if optional_non_empty(tokens.refresh_token.as_str()).is_none()
                    && optional_non_empty(tokens.access_token.as_str()).is_none()
                {
                    return Ok(None);
                }
                let account_id =
                    optional_non_empty(self.credential_drafts.atomgit.account_id.as_str())
                        .map(ToOwned::to_owned);
                let username = optional_non_empty(self.credential_drafts.atomgit.username.as_str())
                    .map(ToOwned::to_owned);
                let user = match (account_id.clone(), username.clone()) {
                    (Some(id), Some(username)) => Some(OAuthUserInfo {
                        id,
                        username,
                        name: optional_non_empty(
                            self.credential_drafts.atomgit.display_name.as_str(),
                        )
                        .map(ToOwned::to_owned),
                        email: optional_non_empty(self.credential_drafts.atomgit.email.as_str())
                            .map(ToOwned::to_owned),
                        avatar_url: optional_non_empty(
                            self.credential_drafts.atomgit.avatar_url.as_str(),
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

    fn active_credential_issuer(&self) -> Option<CredentialIssuer> {
        self.auth_kind.credential_issuer()
    }

    pub(crate) fn active_tokens(&self) -> Option<&ProviderOAuthTokensDraft> {
        self.credential_drafts
            .active_tokens(self.active_credential_issuer())
    }

    fn active_tokens_mut(&mut self) -> Option<&mut ProviderOAuthTokensDraft> {
        self.credential_drafts
            .active_tokens_mut(self.active_credential_issuer())
    }

    pub(crate) fn redirect_uri(&self) -> Option<&str> {
        self.credential_drafts
            .redirect_uri(self.active_credential_issuer())
    }

    pub(crate) fn callback_url(&self) -> Option<&str> {
        self.credential_drafts
            .callback_url(self.active_credential_issuer())
    }

    pub(crate) fn account_id(&self) -> Option<&str> {
        self.credential_drafts
            .account_id(self.active_credential_issuer())
    }

    pub(crate) fn set_redirect_uri(&mut self, value: String) {
        self.credential_drafts
            .set_redirect_uri(self.active_credential_issuer(), value);
    }

    pub(crate) fn set_callback_url(&mut self, value: String) {
        self.credential_drafts
            .set_callback_url(self.active_credential_issuer(), value);
    }

    pub(crate) fn set_refresh_token(&mut self, value: String) {
        if let Some(tokens) = self.active_tokens_mut() {
            tokens.refresh_token = value;
        }
    }

    pub(crate) fn set_access_token(&mut self, value: String) {
        if let Some(tokens) = self.active_tokens_mut() {
            tokens.access_token = value;
        }
    }

    pub(crate) fn set_expires_at_ms(&mut self, value: String) {
        if let Some(tokens) = self.active_tokens_mut() {
            tokens.expires_at_ms = value;
        }
    }

    pub(crate) fn set_account_id(&mut self, value: String) {
        self.credential_drafts
            .set_account_id(self.active_credential_issuer(), value);
    }

    pub(crate) fn supports_interactive_auth(&self) -> bool {
        matches!(
            self.auth_kind,
            ProviderDraftAuthKind::Credential(Some(
                CredentialIssuer::OpenaiChatgpt
                    | CredentialIssuer::GithubCopilot
                    | CredentialIssuer::Gitlab
                    | CredentialIssuer::AtomGit
            ))
        )
    }

    pub(crate) fn supports_saved_model_listing(&self) -> bool {
        match self.auth_kind {
            ProviderDraftAuthKind::Api | ProviderDraftAuthKind::Gitlab => true,
            ProviderDraftAuthKind::Credential(Some(issuer)) => {
                issuer.supports_saved_model_listing()
            }
            ProviderDraftAuthKind::Unset
            | ProviderDraftAuthKind::None
            | ProviderDraftAuthKind::Credential(None)
            | ProviderDraftAuthKind::BedrockSigv4 => false,
        }
    }

    pub(crate) fn tokens_present(&self) -> bool {
        self.active_tokens().is_some_and(|tokens| {
            !tokens.refresh_token.trim().is_empty() || !tokens.access_token.trim().is_empty()
        })
    }

    fn validate_for_adapters(
        &self,
        adapter_ids: &std::collections::BTreeSet<String>,
    ) -> Result<()> {
        let default_adapter = required_trimmed(self.default_adapter.as_str(), "defaults.adapter")?;
        if !self.auth_kind.supports_adapter(default_adapter) {
            return Err(anyhow!(
                "auth {} does not support defaults.adapter `{default_adapter}`; expected one of {}",
                self.auth_kind.label(),
                supported_provider_draft_adapter_list(&self.auth_kind),
            ));
        }

        let incompatible = adapter_ids
            .iter()
            .filter(|adapter_id| !self.auth_kind.supports_adapter(adapter_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !incompatible.is_empty() {
            return Err(anyhow!(
                "auth {} does not support adapter(s): {}; expected one of {}",
                self.auth_kind.label(),
                incompatible.join(", "),
                supported_provider_draft_adapter_list(&self.auth_kind),
            ));
        }

        match self.auth_kind {
            ProviderDraftAuthKind::Unset => {
                return Err(anyhow!("provider auth_mode is required"));
            }
            ProviderDraftAuthKind::None => {}
            ProviderDraftAuthKind::Api => {
                let requires_base_url = adapter_ids.iter().any(|adapter_id| {
                    self.auth_kind
                        .adapter_rule(adapter_id.as_str())
                        .map(|rule| rule.requires_base_url)
                        .unwrap_or(false)
                });
                if requires_base_url && optional_non_empty(self.auth.base_url.as_str()).is_none() {
                    return Err(anyhow!(
                        "api auth requires base_url when using openai, anthropic, or gemini adapters"
                    ));
                }
            }
            ProviderDraftAuthKind::Gitlab => {
                if optional_non_empty(self.auth.api_key.as_str()).is_none()
                    && optional_non_empty(self.auth.api_key_env.as_str()).is_none()
                {
                    return Err(anyhow!("gitlab_api auth requires api_key or api_key_env"));
                }
            }
            ProviderDraftAuthKind::Credential(None) => {
                return Err(anyhow!("credential auth requires credential_issuer"));
            }
            ProviderDraftAuthKind::Credential(Some(issuer)) => {
                if issuer.uses_http_endpoint()
                    && optional_non_empty(self.auth.base_url.as_str()).is_none()
                {
                    return Err(anyhow!(
                        "credential issuer `{}` requires base_url",
                        credential_issuer_label(issuer)
                    ));
                }
                if issuer.requires_service_key_env()
                    && optional_non_empty(self.auth.service_key_env.as_str()).is_none()
                {
                    return Err(anyhow!(
                        "credential issuer `{}` requires service_key_env",
                        credential_issuer_label(issuer)
                    ));
                }
            }
            ProviderDraftAuthKind::BedrockSigv4 => {
                let has_access_key_id =
                    optional_non_empty(self.auth.access_key_id.as_str()).is_some();
                let has_secret_access_key =
                    optional_non_empty(self.auth.secret_access_key.as_str()).is_some();
                if has_access_key_id ^ has_secret_access_key {
                    return Err(anyhow!(
                        "bedrock_sigv4 requires access_key_id and secret_access_key together"
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_for_adapters_for_save(
        &self,
        adapter_ids: &std::collections::BTreeSet<String>,
    ) -> std::result::Result<(), ProviderStudioSaveValidationError> {
        let default_adapter = required_provider_save_field(
            self.default_adapter.as_str(),
            ProviderStudioSaveField::DefaultAdapter,
        )?;
        if !self.auth_kind.supports_adapter(default_adapter) {
            return Err(
                ProviderStudioSaveValidationError::UnsupportedDefaultAdapter {
                    auth_kind: self.auth_kind.clone(),
                    adapter: default_adapter.to_owned(),
                    supported: supported_provider_draft_adapter_list(&self.auth_kind),
                },
            );
        }

        let incompatible = adapter_ids
            .iter()
            .filter(|adapter_id| !self.auth_kind.supports_adapter(adapter_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !incompatible.is_empty() {
            return Err(ProviderStudioSaveValidationError::UnsupportedAdapters {
                auth_kind: self.auth_kind.clone(),
                adapters: incompatible,
                supported: supported_provider_draft_adapter_list(&self.auth_kind),
            });
        }

        match self.auth_kind {
            ProviderDraftAuthKind::Unset => {
                return Err(ProviderStudioSaveValidationError::FieldRequired(
                    ProviderStudioSaveField::AuthMode,
                ));
            }
            ProviderDraftAuthKind::None => {}
            ProviderDraftAuthKind::Api => {
                let requires_base_url = adapter_ids.iter().any(|adapter_id| {
                    self.auth_kind
                        .adapter_rule(adapter_id.as_str())
                        .map(|rule| rule.requires_base_url)
                        .unwrap_or(false)
                });
                if requires_base_url && optional_non_empty(self.auth.base_url.as_str()).is_none() {
                    return Err(ProviderStudioSaveValidationError::ApiBaseUrlRequired);
                }
            }
            ProviderDraftAuthKind::Gitlab => {
                if optional_non_empty(self.auth.api_key.as_str()).is_none()
                    && optional_non_empty(self.auth.api_key_env.as_str()).is_none()
                {
                    return Err(ProviderStudioSaveValidationError::GitlabApiKeyOrEnvRequired);
                }
            }
            ProviderDraftAuthKind::Credential(None) => {
                return Err(ProviderStudioSaveValidationError::FieldRequired(
                    ProviderStudioSaveField::CredentialIssuer,
                ));
            }
            ProviderDraftAuthKind::Credential(Some(issuer)) => {
                if issuer.uses_http_endpoint()
                    && optional_non_empty(self.auth.base_url.as_str()).is_none()
                {
                    return Err(
                        ProviderStudioSaveValidationError::CredentialBaseUrlRequired { issuer },
                    );
                }
                if issuer.requires_service_key_env()
                    && optional_non_empty(self.auth.service_key_env.as_str()).is_none()
                {
                    return Err(
                        ProviderStudioSaveValidationError::CredentialServiceKeyEnvRequired {
                            issuer,
                        },
                    );
                }
            }
            ProviderDraftAuthKind::BedrockSigv4 => {
                let has_access_key_id =
                    optional_non_empty(self.auth.access_key_id.as_str()).is_some();
                let has_secret_access_key =
                    optional_non_empty(self.auth.secret_access_key.as_str()).is_some();
                if has_access_key_id ^ has_secret_access_key {
                    return Err(ProviderStudioSaveValidationError::BedrockKeyPairRequired);
                }
            }
        }

        Ok(())
    }

    fn validate_listing_request(&self, adapter_ids: &[String]) -> Result<()> {
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
                self.auth_kind
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
        self.validate_for_adapters(
            &selected
                .into_iter()
                .map(ToOwned::to_owned)
                .collect::<std::collections::BTreeSet<_>>(),
        )
    }

    fn build_listing_target(
        &self,
        adapter_ids: &[String],
    ) -> Result<agena::config::ProviderAdapterModelsTarget> {
        if !self.auth_kind.supports_draft_model_listing() {
            return Err(anyhow!(
                "draft adapter model listing requires api, gitlab_api, or atomgit credential auth; current auth is {}",
                self.auth_kind.label()
            ));
        }
        self.validate_listing_request(adapter_ids)?;
        match self.auth_kind {
            ProviderDraftAuthKind::Api => draft_provider_adapter_models_target(
                Some(self.provider_id.as_str()),
                self.auth.base_url.as_str(),
                agena::config::ProviderProtocolPathsConfig::default(),
                Some(self.auth.api_key.as_str()),
                Some(self.auth.api_key_env.as_str()),
                adapter_ids,
            )
            .map_err(map_provider_adapter_models_config_error),
            ProviderDraftAuthKind::Gitlab => draft_gitlab_provider_adapter_models_target(
                Some(self.provider_id.as_str()),
                Some(self.auth.api_key.as_str()),
                Some(self.auth.api_key_env.as_str()),
                adapter_ids,
            )
            .map_err(map_provider_adapter_models_config_error),
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
                let credential = self.oauth_auth_data()?.ok_or_else(|| {
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
                    Some(self.provider_id.as_str()),
                    credential,
                    adapter_ids,
                )
                .map_err(map_provider_adapter_models_config_error)
            }
            _ => unreachable!("listing guard ensures only supported draft auth kinds reach here"),
        }
    }

    pub(crate) fn request_fingerprint(&self, adapter_ids: &[String]) -> String {
        let mut hasher = DefaultHasher::new();
        self.source_provider_id
            .as_deref()
            .unwrap_or("<new>")
            .trim()
            .hash(&mut hasher);
        self.provider_id.trim().hash(&mut hasher);
        self.auth_kind.label().hash(&mut hasher);
        self.auth.base_url.trim().hash(&mut hasher);
        self.auth.instance_url.trim().hash(&mut hasher);
        self.auth.api_key_env.trim().hash(&mut hasher);
        self.auth.api_key.trim().hash(&mut hasher);
        self.auth.credential_issuer.trim().hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .redirect_uri
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .callback_url
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .tokens
            .refresh_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .tokens
            .access_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .tokens
            .expires_at_ms
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .account_id
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .github_copilot
            .enterprise_domain
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .github_copilot
            .tokens
            .refresh_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .github_copilot
            .tokens
            .access_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .github_copilot
            .tokens
            .expires_at_ms
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .redirect_uri
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .callback_url
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .tokens
            .refresh_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .tokens
            .access_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .tokens
            .expires_at_ms
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .atomgit
            .tokens
            .refresh_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .atomgit
            .tokens
            .access_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .atomgit
            .tokens
            .expires_at_ms
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .atomgit
            .account_id
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .atomgit
            .username
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .atomgit
            .display_name
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .atomgit
            .email
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .atomgit
            .avatar_url
            .trim()
            .hash(&mut hasher);
        self.auth.region.trim().hash(&mut hasher);
        self.auth.profile.trim().hash(&mut hasher);
        self.auth.access_key_id.trim().hash(&mut hasher);
        self.auth.secret_access_key.trim().hash(&mut hasher);
        self.auth.session_token.trim().hash(&mut hasher);
        self.auth.service_key_env.trim().hash(&mut hasher);
        self.default_adapter.trim().hash(&mut hasher);
        self.default_model.trim().hash(&mut hasher);
        let mut normalized_adapter_ids = adapter_ids
            .iter()
            .map(|adapter_id| adapter_id.trim())
            .filter(|adapter_id| !adapter_id.is_empty())
            .collect::<Vec<_>>();
        normalized_adapter_ids.sort_unstable();
        normalized_adapter_ids.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
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

    pub async fn delete_config_setting(&self, path: &str) -> Result<ConfigSettingsEditResponse> {
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

    pub fn provider_config_draft(&self, provider_id: Option<&str>) -> Result<ProviderConfigDraft> {
        let Some(provider_id) = provider_id.map(str::trim).filter(|value| !value.is_empty()) else {
            let mut draft = ProviderConfigDraft::new_empty();
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
        Ok(ProviderConfigDraft::from_resolved(provider_id, provider))
    }

    pub async fn start_provider_draft_auth(
        &self,
        draft: ProviderConfigDraft,
    ) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
        start_provider_draft_auth(draft).await
    }

    pub async fn continue_provider_draft_auth(
        &self,
        draft: ProviderConfigDraft,
    ) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
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

        self.runtime
            .current_snapshot()
            .resolve_default_model()
            .context("failed to resolve default model selection")?
            .ok_or_else(|| anyhow!("no providers configured"))
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
        let target = draft.build_listing_target(adapter_ids)?;
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
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_provider_save_field(
            draft.provider_id.as_str(),
            ProviderStudioSaveField::ProviderId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let requested_default_adapter =
            optional_non_empty(draft.default_adapter.as_str()).map(str::to_owned);
        let requested_default_model =
            optional_non_empty(draft.default_model.as_str()).map(str::to_owned);
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, selected_adapter_ids);
        draft
            .validate_for_adapters_for_save(&effective_adapter_ids)
            .map_err(ProviderStudioSaveError::Validation)?;

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
            .read_file_provider_settings(provider_id)
            .map_err(ProviderStudioSaveError::other)?
            .unwrap_or_else(|| JsonValue::Object(JsonMap::new()));
        let provider_object = provider_value
            .as_object_mut()
            .ok_or(ProviderStudioSaveError::ExistingProviderSettingsMustBeObject)?;
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
            let adapter_object = adapter_value.as_object_mut().ok_or_else(|| {
                ProviderStudioSaveError::ProviderAdapterMustBeObject {
                    adapter_id: adapter_id.to_owned(),
                }
            })?;
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
                        provider_model_json_for_model_id_with_draft(
                            &draft,
                            adapter_id,
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

        let (default_adapter, default_model) = resolve_provider_defaults_from_value_for_save(
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
        let default_model_value =
            provider_model_overlay_to_json(draft.apply_native_tools_to_model_overlay(
                default_adapter.as_str(),
                default_model.as_str(),
                serde_json::from_value(default_model_value).unwrap_or_default(),
            ));
        adapters
            .entry(default_adapter.clone())
            .or_insert_with(|| json!({ "enabled": true }));
        ensure_provider_model_entry(
            adapters
                .get_mut(default_adapter.as_str())
                .expect("default adapter must exist"),
            default_model.as_str(),
            default_model_value,
        )
        .map_err(ProviderStudioSaveError::other)?;

        provider_object.insert("enabled".to_owned(), JsonValue::Bool(true));
        provider_object.insert(
            "defaults".to_owned(),
            json!({
                "adapter": default_adapter,
                "model": default_model,
            }),
        );
        provider_object.insert(
            "auth".to_owned(),
            JsonValue::Object(build_provider_auth_patch_value_for_save(&draft)?),
        );
        provider_object.insert("adapters".to_owned(), JsonValue::Object(adapters));
        self.set_provider_settings(provider_id, provider_value)
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::ProviderDraftSaved {
            provider_id: provider_id.to_owned(),
            default_adapter,
            default_model,
        })
    }

    pub async fn save_provider_adapter_matches(
        &self,
        draft: ProviderConfigDraft,
        adapter_models: ProviderAdapterModelsResource,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_provider_save_field(
            draft.provider_id.as_str(),
            ProviderStudioSaveField::ProviderId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let adapter_id = required_provider_save_field(
            adapter_models.adapter_id.as_str(),
            ProviderStudioSaveField::AdapterId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, &[adapter_id.to_owned()]);
        draft
            .validate_for_adapters_for_save(&effective_adapter_ids)
            .map_err(ProviderStudioSaveError::Validation)?;
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
                    provider_model_json_for_model_id_with_draft(
                        &draft,
                        adapter_id,
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
        let provider_patch = build_provider_patch_value_for_save(
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
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::AdapterMatchesSaved {
            provider_id: provider_id.to_owned(),
            adapter_id: adapter_id.to_owned(),
            listed_model_count: adapter_models.models.len(),
            matched_model_count,
        })
    }

    pub async fn save_provider_model(
        &self,
        draft: ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        provider_model: Option<ProviderModel>,
        set_default: bool,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_provider_save_field(
            draft.provider_id.as_str(),
            ProviderStudioSaveField::ProviderId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let adapter_id =
            required_provider_save_field(adapter_id, ProviderStudioSaveField::AdapterId)
                .map_err(ProviderStudioSaveError::Validation)?;
        let model_id = required_provider_save_field(model_id, ProviderStudioSaveField::ModelId)
            .map_err(ProviderStudioSaveError::Validation)?;
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, &[adapter_id.to_owned()]);
        draft
            .validate_for_adapters_for_save(&effective_adapter_ids)
            .map_err(ProviderStudioSaveError::Validation)?;
        let catalog_entries =
            self.lookup_model_catalog_entries(&[catalog_lookup_id_for_model_id(model_id)]);
        let model_value = provider_model_json_for_model_id_with_draft(
            &draft,
            adapter_id,
            &catalog_entries,
            model_id,
            provider_model.as_ref(),
        );
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
        let provider_patch = build_provider_patch_value_for_save(
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
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::ModelSaved {
            provider_id: provider_id.to_owned(),
            adapter_id: adapter_id.to_owned(),
            model_id: model_id.to_owned(),
        })
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
                    target: ConfigSettingsPathInput { path: Some(path) },
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
                target: ConfigSettingsPathInput {
                    path: Some(provider_settings_path(provider_id)),
                },
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
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        let mut draft = draft;
        draft.normalize_shape();
        let provider_id = required_provider_save_field(
            draft.provider_id.as_str(),
            ProviderStudioSaveField::ProviderId,
        )
        .map_err(ProviderStudioSaveError::Validation)?;
        let adapter_id =
            required_provider_save_field(adapter_id, ProviderStudioSaveField::AdapterId)
                .map_err(ProviderStudioSaveError::Validation)?;
        let model_id = required_provider_save_field(model_id, ProviderStudioSaveField::ModelId)
            .map_err(ProviderStudioSaveError::Validation)?;
        let JsonValue::Object(_) = &model_value else {
            return Err(ProviderStudioSaveError::ProviderModelConfigMustBeObject);
        };
        let effective_adapter_ids =
            self.effective_provider_draft_adapter_ids(&draft, &[adapter_id.to_owned()]);
        draft
            .validate_for_adapters_for_save(&effective_adapter_ids)
            .map_err(ProviderStudioSaveError::Validation)?;
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
        let include_defaults = set_default || draft.source_provider_id.is_none();
        let existing_adapter = draft
            .source_provider_id
            .as_deref()
            .map(|provider_id| {
                read_file_setting(
                    self.runtime.config_resolution().meta.config_path.clone(),
                    ConfigSettingsGetInput {
                        target: ConfigSettingsPathInput {
                            path: Some(provider_adapter_settings_path(provider_id, adapter_id)),
                        },
                        source: agena::config::ConfigSettingsSource::File,
                    },
                )
                .map_err(ProviderStudioSaveError::other)
                .map(|response| response.value)
            })
            .transpose()?;
        let model_overlay = serde_json::from_value::<ProviderModelOverlay>(model_value)
            .map_err(ProviderStudioSaveError::other)?;
        let adapter_patch = merge_provider_model_adapter_patch_for_save(
            existing_adapter,
            model_id,
            provider_model_overlay_to_json(draft.apply_native_tools_to_model_overlay(
                adapter_id,
                model_id,
                model_overlay,
            )),
        )?;
        let mut provider_patch = JsonMap::new();
        provider_patch.insert("enabled".to_owned(), JsonValue::Bool(true));
        provider_patch.insert(
            "auth".to_owned(),
            JsonValue::Object(build_provider_auth_patch_value_for_save(&draft)?),
        );
        provider_patch.insert(
            "adapters".to_owned(),
            json!({
                adapter_id: adapter_patch,
            }),
        );
        if include_defaults {
            provider_patch.insert(
                "defaults".to_owned(),
                json!({
                    "adapter": default_adapter,
                    "model": default_model,
                }),
            );
        }
        self.patch_provider_settings(provider_id, JsonValue::Object(provider_patch))
            .await
            .map_err(ProviderStudioSaveError::other)?;
        Ok(ProviderStudioSaveResult::ConfiguredModelSaved {
            provider_id: provider_id.to_owned(),
            adapter_id: adapter_id.to_owned(),
            model_id: model_id.to_owned(),
        })
    }

    async fn list_provider_adapter_models_with_target(
        &self,
        target: agena::config::ProviderAdapterModelsTarget,
    ) -> Result<ProviderAdapterModelsResponse> {
        let resolution = self.runtime.config_resolution();
        let adapter_models = list_provider_adapter_models_with_config(
            &resolution.config,
            &target,
            &ProcessEnvironment,
        )
        .await
        .context("failed to list provider adapter models")?;
        Ok(ProviderAdapterModelsResponse {
            provider_id: adapter_models.provider_id,
            adapters: adapter_models
                .adapters
                .into_iter()
                .map(Into::into)
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
                target: ConfigSettingsPathInput {
                    path: Some("providers".to_owned()),
                },
                changes: json!({
                    provider_id: provider_patch,
                }),
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
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
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
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
                .list_messages_with_parts(session_id, cursor.clone(), 200, PartLoadMode::Full)
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

    pub async fn get_session_permission_studio_state(
        &self,
        session_id: i64,
    ) -> Result<SessionPermissionStudioState> {
        let execution = self.get_session_state(session_id).await?;
        let session = self
            .session_manager()?
            .get_session(session_id)
            .await
            .with_context(|| format!("failed to load session {session_id}"))?;
        Ok(SessionPermissionStudioState {
            session_title: execution.session.title.clone(),
            permission: session.runtime().execution.selection.permission.clone(),
            effective_permission: execution.execution.effective_permission.clone(),
        })
    }

    pub async fn list_messages(
        &self,
        session_id: i64,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<PaginatedResponse<MessageResource>> {
        // The transcript view should render history with the same shape and
        // styling as live events, so it always loads full parts here.
        self.list_messages_with_parts(session_id, cursor, limit, PartLoadMode::Full)
            .await
    }

    pub async fn list_messages_with_parts(
        &self,
        session_id: i64,
        cursor: Option<String>,
        limit: u64,
        parts: PartLoadMode,
    ) -> Result<PaginatedResponse<MessageResource>> {
        match dispatch::dispatch_query(
            &self.app_state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor,
                limit: Some(limit),
                parts,
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
            .list_messages_with_parts(session_id, None, latest_message_limit, PartLoadMode::Full)
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
                        | EventKind::RunCompleted(_)
                        | EventKind::RunAborted(_)
                        | EventKind::SystemNoticeAppended(_)
                        | EventKind::ExecutionFailed(_)
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

    pub async fn submit_parts_message_with_options(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
        request: RunOptions,
    ) -> Result<SessionExecutionResource> {
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::SubmitMessage(SubmitMessageParams {
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
        .context("failed to submit user message")
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
        match dispatch::dispatch_command(
            &self.app_state,
            ApiCommand::CompactSession(CompactSessionParams {
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
        .context("failed to compact session")
    }

    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: agena::agent::PermissionConfig,
    ) -> Result<SessionExecutionResource> {
        self.session_manager()?
            .set_session_permission(session_id, permission)
            .await
            .with_context(|| format!("failed to set permission for session {session_id}"))?;
        self.get_session_state(session_id).await
    }

    /// Best-effort cancel of the in-flight run for `session_id`. Forwards
    /// to `SessionManager::cancel_active_run`; the manager owns the
    /// `CancellationToken` for the spawned run task. If no run is
    /// active this is a no-op.
    pub async fn cancel_run(&self, session_id: i64) -> Result<()> {
        self.session_manager()?
            .cancel_active_run(session_id)
            .await
            .context("failed to cancel active run")
    }

    /// Inject `parts` as a steer message into the in-flight run. Returns
    /// `Err` when there is no active run or the run is in a phase that
    /// no longer accepts steers (the caller should re-queue).
    pub async fn steer_input(&self, session_id: i64, parts: Vec<PartContent>) -> Result<()> {
        self.session_manager()?
            .steer_input(session_id, parts)
            .await
            .context("failed to steer run")
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
) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
    draft.normalize_shape();
    match draft.auth_kind {
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            let redirect_uri = required_provider_auth_field(
                draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                ProviderDraftAuthField::RedirectUri,
            )?;
            let start =
                start_openai_browser_oauth(redirect_uri).map_err(ProviderDraftAuthError::other)?;
            draft.credential_drafts.openai_chatgpt.callback_url.clear();
            draft.credential_drafts.openai_chatgpt.browser =
                Some(ProviderBrowserAuthSessionDraft {
                    authorize_url: start.authorize_url.clone(),
                    state: start.state.clone(),
                    pkce_verifier: start.pkce_verifier,
                });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: ProviderDraftAuthMessage::OpenaiBrowserStarted,
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
            let start = start_copilot_device_code(domain)
                .await
                .map_err(ProviderDraftAuthError::other)?;
            draft.credential_drafts.github_copilot.device = Some(ProviderDeviceAuthSessionDraft {
                verification_url: start.verification_url.clone(),
                user_code: start.user_code.clone(),
                device_code: start.device_code,
                interval_seconds: start.interval_seconds,
            });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: ProviderDraftAuthMessage::CopilotDeviceStarted {
                    user_code: start.user_code,
                },
                clipboard_text: Some(start.verification_url),
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            let instance_url = required_provider_auth_field(
                draft.auth.instance_url.as_str(),
                ProviderDraftAuthField::InstanceUrl,
            )?;
            let redirect_uri = required_provider_auth_field(
                draft.credential_drafts.gitlab.redirect_uri.as_str(),
                ProviderDraftAuthField::RedirectUri,
            )?;
            let start = start_gitlab_oauth(instance_url, redirect_uri)
                .map_err(ProviderDraftAuthError::other)?;
            draft.credential_drafts.gitlab.callback_url.clear();
            draft.credential_drafts.gitlab.browser = Some(ProviderBrowserAuthSessionDraft {
                authorize_url: start.authorize_url.clone(),
                state: start.state.clone(),
                pkce_verifier: start.pkce_verifier,
            });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: ProviderDraftAuthMessage::GitlabBrowserStarted,
                clipboard_text: Some(start.authorize_url),
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
            let start = start_atomgit_oauth()
                .await
                .map_err(ProviderDraftAuthError::other)?;
            draft.credential_drafts.atomgit.browser = Some(ProviderBrowserAuthSessionDraft {
                authorize_url: start.authorize_url.clone(),
                state: start.state,
                pkce_verifier: String::new(),
            });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: ProviderDraftAuthMessage::AtomGitBrowserStarted,
                clipboard_text: Some(start.authorize_url),
            })
        }
        _ => Err(ProviderDraftAuthError::UnsupportedInteractiveLogin),
    }
}

fn required_provider_auth_field<'a>(
    value: &'a str,
    field: ProviderDraftAuthField,
) -> std::result::Result<&'a str, ProviderDraftAuthError> {
    optional_non_empty(value).ok_or(ProviderDraftAuthError::RequiredField(field))
}

async fn continue_provider_draft_auth(
    mut draft: ProviderConfigDraft,
) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
    draft.normalize_shape();
    match draft.auth_kind {
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            let session = draft
                .credential_drafts
                .openai_chatgpt
                .browser
                .clone()
                .ok_or(ProviderDraftAuthError::StartBrowserAuthFirst)?;
            let redirect_uri = required_provider_auth_field(
                draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                ProviderDraftAuthField::RedirectUri,
            )?;
            let callback_url = required_provider_auth_field(
                draft.credential_drafts.openai_chatgpt.callback_url.as_str(),
                ProviderDraftAuthField::CallbackUrl,
            )?;
            let callback = parse_oauth_callback_url(callback_url, Some(session.state.as_str()))
                .map_err(ProviderDraftAuthError::other)?;
            let token = exchange_openai_oauth_code(
                callback.code.as_str(),
                session.pkce_verifier.as_str(),
                redirect_uri,
            )
            .await
            .map_err(ProviderDraftAuthError::other)?;
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
                message: ProviderDraftAuthMessage::OpenaiCredentialCaptured,
                clipboard_text: None,
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
            let session = draft
                .credential_drafts
                .github_copilot
                .device
                .clone()
                .ok_or(ProviderDraftAuthError::StartDeviceAuthFirst)?;
            let domain = optional_non_empty(
                draft
                    .credential_drafts
                    .github_copilot
                    .enterprise_domain
                    .as_str(),
            )
            .unwrap_or("github.com");
            let Some(token) = poll_copilot_device_code(domain, session.device_code.as_str())
                .await
                .map_err(ProviderDraftAuthError::other)?
            else {
                return Ok(ProviderDraftAuthActionResult {
                    draft,
                    message: ProviderDraftAuthMessage::CopilotPending,
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
                message: ProviderDraftAuthMessage::CopilotCredentialCaptured,
                clipboard_text: None,
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            let session = draft
                .credential_drafts
                .gitlab
                .browser
                .clone()
                .ok_or(ProviderDraftAuthError::StartBrowserAuthFirst)?;
            let instance_url = required_provider_auth_field(
                draft.auth.instance_url.as_str(),
                ProviderDraftAuthField::InstanceUrl,
            )?;
            let redirect_uri = required_provider_auth_field(
                draft.credential_drafts.gitlab.redirect_uri.as_str(),
                ProviderDraftAuthField::RedirectUri,
            )?;
            let callback_url = required_provider_auth_field(
                draft.credential_drafts.gitlab.callback_url.as_str(),
                ProviderDraftAuthField::CallbackUrl,
            )?;
            let callback = parse_oauth_callback_url(callback_url, Some(session.state.as_str()))
                .map_err(ProviderDraftAuthError::other)?;
            let token = exchange_gitlab_oauth_code(
                instance_url,
                callback.code.as_str(),
                session.pkce_verifier.as_str(),
                redirect_uri,
            )
            .await
            .map_err(ProviderDraftAuthError::other)?;
            update_oauth_tokens_from_response(&mut draft.credential_drafts.gitlab.tokens, &token);
            draft.credential_drafts.gitlab.callback_url.clear();
            draft.credential_drafts.gitlab.browser = None;
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: ProviderDraftAuthMessage::GitlabCredentialCaptured,
                clipboard_text: None,
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit)) => {
            let session = draft
                .credential_drafts
                .atomgit
                .browser
                .clone()
                .ok_or(ProviderDraftAuthError::StartBrowserAuthFirst)?;
            if !poll_atomgit_oauth_state(session.state.as_str())
                .await
                .map_err(ProviderDraftAuthError::other)?
            {
                return Ok(ProviderDraftAuthActionResult {
                    draft,
                    message: ProviderDraftAuthMessage::AtomGitPending,
                    clipboard_text: None,
                });
            }

            let token = exchange_atomgit_oauth_state(session.state.as_str())
                .await
                .map_err(ProviderDraftAuthError::other)?;
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
                message: ProviderDraftAuthMessage::AtomGitCredentialCaptured,
                clipboard_text: None,
            })
        }
        _ => Err(ProviderDraftAuthError::UnsupportedInteractiveLogin),
    }
}

fn local_model_catalog_summary(
    catalog: &agena::model_catalog::ModelCatalogResponse,
) -> LocalModelCatalogResponse {
    LocalModelCatalogResponse {
        refreshing: false,
        last_refresh_at: catalog.last_refresh_at,
        last_successful_source: catalog.last_successful_source,
        last_error: catalog.last_error.clone(),
        entry_count: catalog.entries.len(),
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
        match entry.source {
            ModelCatalogSourceKind::Generated => "generated".to_owned(),
            ModelCatalogSourceKind::Cache => "cache".to_owned(),
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
        .min_by_key(|entry| entry.model_id.as_str())
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
    provider_model_overlay_to_json(provider_model_overlay_for_model_id(
        catalog_entries,
        model_id,
        provider_model,
    ))
}

fn provider_model_json_for_model_id_with_draft(
    draft: &ProviderConfigDraft,
    adapter_id: &str,
    catalog_entries: &[ModelCatalogEntryResource],
    model_id: &str,
    provider_model: Option<&ProviderModel>,
) -> JsonValue {
    provider_model_overlay_to_json(draft.apply_native_tools_to_model_overlay(
        adapter_id,
        model_id,
        provider_model_overlay_for_model_id(catalog_entries, model_id, provider_model),
    ))
}

fn provider_model_overlay_for_model_id(
    catalog_entries: &[ModelCatalogEntryResource],
    model_id: &str,
    provider_model: Option<&ProviderModel>,
) -> ProviderModelOverlay {
    preferred_catalog_entry_for_model_id(catalog_entries, model_id)
        .or_else(|| {
            let lookup_id = catalog_lookup_id_for_model_id(model_id);
            (lookup_id != model_id)
                .then(|| preferred_catalog_entry_for_model_id(catalog_entries, lookup_id.as_str()))
                .flatten()
        })
        .map(catalog_entry_to_provider_model_overlay)
        .or_else(|| {
            provider_model.and_then(|provider_model| {
                preferred_catalog_entry_for_provider_model(catalog_entries, provider_model)
                    .map(catalog_entry_to_provider_model_overlay)
                    .or_else(|| Some(provider_model_to_provider_model_overlay(provider_model)))
            })
        })
        .unwrap_or_default()
}

fn provider_model_overlay_to_json(overlay: ProviderModelOverlay) -> JsonValue {
    if overlay.definition.is_empty() {
        return JsonValue::Object(JsonMap::new());
    }

    match serde_json::to_value(overlay) {
        Ok(JsonValue::Object(mut value)) => {
            if matches!(value.get("enabled"), Some(JsonValue::Bool(true))) {
                value.remove("enabled");
            }
            JsonValue::Object(value)
        }
        Ok(other) => other,
        Err(_) => JsonValue::Object(JsonMap::new()),
    }
}

fn catalog_entry_to_provider_model_overlay(
    entry: &ModelCatalogEntryResource,
) -> ProviderModelOverlay {
    provider_model_overlay_from_catalog_definition(&catalog_entry_to_catalog_definition(entry))
}

fn catalog_entry_to_catalog_definition(
    entry: &ModelCatalogEntryResource,
) -> CatalogModelDefinition {
    let mut definition = CatalogModelDefinition::default();
    definition.lifecycle = entry.lifecycle;
    definition.context_window_tokens = entry.context_window_tokens;
    definition.max_input_tokens = entry.max_input_tokens;
    definition.max_output_tokens = entry.max_output_tokens;
    definition.description = entry.description.clone();
    definition.knowledge_cutoff = entry.knowledge_cutoff.clone();
    definition.release_date = entry.release_date.clone();
    definition.last_updated = entry.last_updated.clone();
    definition.open_weights = entry.open_weights;
    definition.default_thinking_mode = entry.default_thinking_mode.clone();
    definition.supports_parallel_tool_calls = entry.supports_parallel_tool_calls;
    definition.supports_verbosity = entry.supports_verbosity;
    definition.default_verbosity = entry.default_verbosity.clone();
    definition.default_temperature = entry.default_temperature.clone();
    definition.default_top_p = entry.default_top_p.clone();
    definition.default_top_k = entry.default_top_k;
    definition.assistant_reasoning_interleaved = entry.assistant_reasoning_interleaved;
    definition.assistant_reasoning_field = entry.assistant_reasoning_field.clone();
    definition.output_modalities = entry.output_modalities.clone();
    definition.pricing = entry.pricing.clone();
    definition.display_name = entry.display_name.clone();
    definition.origin = entry.origin.clone();
    definition.thinking_modes = entry.thinking_modes.clone();
    definition.speed_modes = entry.speed_modes.clone();
    definition.capabilities = sanitized_catalog_capability_patch(&entry.capabilities);
    definition
}

fn sanitized_catalog_capability_patch(
    patch: &agena::provider::ModelCapabilityPatch,
) -> agena::provider::ModelCapabilityPatch {
    let mut patch = patch.clone();
    patch.input = sanitize_selection_patch(patch.input.take());
    patch.features = sanitize_selection_patch(patch.features.take());

    patch
}

fn sanitize_selection_patch<T: Clone + PartialEq>(
    patch: Option<agena::provider::CapabilitySelectionPatch<T>>,
) -> Option<agena::provider::CapabilitySelectionPatch<T>> {
    let patch = patch?;
    match patch {
        agena::provider::CapabilitySelectionPatch::Supported(mut supported) => {
            dedupe_vec(&mut supported);
            agena::provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
                supported,
                Vec::new(),
            )
        }
        agena::provider::CapabilitySelectionPatch::Patch(mut values) => {
            dedupe_vec(&mut values.supported);
            dedupe_vec(&mut values.unsupported);
            values
                .unsupported
                .retain(|value| !values.supported.contains(value));
            agena::provider::CapabilitySelectionPatch::optional_from_supported_unsupported(
                values.supported,
                values.unsupported,
            )
        }
    }
}

fn provider_model_to_provider_model_overlay(model: &ProviderModel) -> ProviderModelOverlay {
    provider_model_overlay_from_catalog_definition(&catalog_definition_from_model(model))
}

fn provider_model_route_id(adapter_id: &str, model_id: &str) -> String {
    format!("{adapter_id}/{model_id}")
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

fn auth_data_has_access_or_api_key(auth: &AuthData) -> bool {
    match auth {
        AuthData::Api { key } | AuthData::WellKnown { key, .. } => !key.trim().is_empty(),
        AuthData::OAuth { access, .. } => !access.trim().is_empty(),
    }
}

fn build_provider_patch_value_for_save(
    draft: &ProviderConfigDraft,
    default_adapter: &str,
    default_model: &str,
    adapters: JsonValue,
    include_defaults: bool,
) -> std::result::Result<JsonValue, ProviderStudioSaveError> {
    let adapters = serde_json::from_value::<
        std::collections::BTreeMap<String, ProviderAdapterOverlay>,
    >(adapters)
    .map_err(ProviderStudioSaveError::other)?;
    let overlay = draft.to_provider_overlay_for_save(
        default_adapter,
        default_model,
        adapters,
        include_defaults,
    )?;
    serde_json::to_value(overlay).map_err(ProviderStudioSaveError::other)
}

fn build_provider_auth_patch_value_for_save(
    draft: &ProviderConfigDraft,
) -> std::result::Result<JsonMap<String, JsonValue>, ProviderStudioSaveError> {
    serde_json::to_value(draft.to_auth_overlay_for_save()?)
        .map_err(ProviderStudioSaveError::other)
        .and_then(|value| match value {
            JsonValue::Object(object) => Ok(object),
            _ => Err(ProviderStudioSaveError::other(
                "provider auth overlay must serialize as an object",
            )),
        })
}

fn provider_model_settings_path(provider_id: &str, adapter_id: &str, model_id: &str) -> String {
    format!(
        "providers.{}.adapters.{}.models.{}",
        quoted_settings_segment(provider_id),
        quoted_settings_segment(adapter_id),
        quoted_settings_segment(model_id),
    )
}

fn provider_adapter_settings_path(provider_id: &str, adapter_id: &str) -> String {
    format!(
        "providers.{}.adapters.{}",
        quoted_settings_segment(provider_id),
        quoted_settings_segment(adapter_id),
    )
}

fn provider_settings_path(provider_id: &str) -> String {
    format!("providers.{}", quoted_settings_segment(provider_id))
}

fn merge_provider_model_adapter_patch_for_save(
    existing_adapter: Option<JsonValue>,
    model_id: &str,
    model_value: JsonValue,
) -> std::result::Result<JsonValue, ProviderStudioSaveError> {
    let mut adapter = match existing_adapter {
        Some(JsonValue::Object(object)) => object,
        Some(JsonValue::Null) | None => JsonMap::new(),
        Some(_) => {
            return Err(ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject);
        }
    };
    adapter.insert("enabled".to_owned(), JsonValue::Bool(true));
    let models = adapter
        .entry("models".to_owned())
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    let Some(models_object) = models.as_object_mut() else {
        return Err(ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject);
    };
    models_object.insert(model_id.to_owned(), model_value);
    Ok(JsonValue::Object(adapter))
}

fn provider_model_selection_contains(
    selected_model_keys: &std::collections::BTreeSet<String>,
    adapter_id: &str,
    model_id: &str,
) -> bool {
    selected_model_keys.contains(format!("{adapter_id}\u{1f}{model_id}").as_str())
}

fn resolve_provider_defaults_from_value_for_save(
    adapters: &JsonMap<String, JsonValue>,
    requested_default_adapter: Option<&str>,
    requested_default_model: Option<&str>,
) -> std::result::Result<(String, String), ProviderStudioSaveError> {
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

    Err(ProviderStudioSaveError::Validation(
        ProviderStudioSaveValidationError::SelectAtLeastOneModel,
    ))
}

fn required_provider_save_field<'a>(
    value: &'a str,
    field: ProviderStudioSaveField,
) -> std::result::Result<&'a str, ProviderStudioSaveValidationError> {
    optional_non_empty(value).ok_or(ProviderStudioSaveValidationError::FieldRequired(field))
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

fn trimmed_owned(value: &str) -> Option<String> {
    non_empty(Some(value)).map(ToOwned::to_owned)
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
    use agena::{config::LoadConfigRequest, memory, tracing as tracing_config};
    use std::{collections::BTreeSet, sync::Arc};
    use tempfile::tempdir;

    #[test]
    fn merge_provider_model_adapter_patch_preserves_existing_models() {
        let merged = merge_provider_model_adapter_patch_for_save(
            Some(json!({
                "enabled": true,
                "models": {
                    "model-a": { "display_name": "Model A" },
                    "model-b": { "display_name": "Model B" }
                },
                "base_url": "https://example.com"
            })),
            "model-b",
            json!({ "display_name": "Model B2" }),
        )
        .expect("adapter patch should merge");

        assert_eq!(
            merged,
            json!({
                "enabled": true,
                "models": {
                    "model-a": { "display_name": "Model A" },
                    "model-b": { "display_name": "Model B2" }
                },
                "base_url": "https://example.com"
            })
        );
    }

    #[test]
    fn provider_draft_suggests_official_native_tools_explicitly() {
        let mut draft = ProviderConfigDraft::new_empty();
        draft.auth_kind = ProviderDraftAuthKind::Api;
        draft.auth.base_url = "https://api.openai.com".to_owned();
        draft.default_adapter = "openai".to_owned();
        draft.default_model = "gpt-5".to_owned();
        draft.normalize_shape();

        assert_eq!(
            draft.native_tools_preset,
            ProviderNativeToolsPreset::OpenAiHostedDefaults
        );
        let config = draft.effective_native_tools_config();
        assert!(config.enabled);
        assert_eq!(
            config.routes.web_search,
            Some(ProviderNativeToolRoute::ProviderHosted)
        );
        assert_eq!(
            config.routes.file_search,
            Some(ProviderNativeToolRoute::ProviderHosted)
        );
        assert_eq!(
            config.routes.code_execution,
            Some(ProviderNativeToolRoute::ProviderHosted)
        );
        assert_eq!(config.routes.image_generation, None);
    }

    #[test]
    fn manual_native_tool_disable_stays_disabled() {
        let mut draft = ProviderConfigDraft::new_empty();
        draft.auth_kind = ProviderDraftAuthKind::Api;
        draft.auth.base_url = "https://api.openai.com".to_owned();
        draft.default_adapter = "openai".to_owned();
        draft.default_model = "gpt-5".to_owned();
        draft.normalize_shape();
        draft.set_native_tools_preset(ProviderNativeToolsPreset::Disabled);

        draft.auth.base_url = "https://api.openai.com".to_owned();
        draft.sync_native_tools_suggestion();

        assert_eq!(
            draft.native_tools_preset,
            ProviderNativeToolsPreset::Disabled
        );
        assert!(!draft.effective_native_tools_config().enabled);
    }

    #[test]
    fn existing_provider_draft_does_not_auto_suggest_native_tools() {
        let mut draft = ProviderConfigDraft::new_empty();
        draft.source_provider_id = Some("saved-provider".to_owned());
        draft.auth_kind = ProviderDraftAuthKind::Api;
        draft.auth.base_url = "https://api.openai.com".to_owned();
        draft.default_adapter = "openai".to_owned();
        draft.default_model = "gpt-5".to_owned();
        draft.sync_native_tools_suggestion();

        assert_eq!(
            draft.native_tools_preset,
            ProviderNativeToolsPreset::Disabled
        );
        assert!(!draft.native_tools_touched);
    }

    #[tokio::test]
    async fn plugin_statuses_include_all_builtin_runtime_plugins() {
        let temp = tempdir().expect("temp workspace");
        let config_path = temp.path().join("config.json");
        fs::write(&config_path, "{}").expect("write empty config");
        let db = tracing_config::connect_database("sqlite::memory:", &Default::default())
            .await
            .expect("connect sqlite");
        let runtime = AgenaRuntime::builder()
            .with_load_request(LoadConfigRequest {
                config_path: Some(config_path),
                overrides: Vec::new(),
            })
            .with_workspace_root(temp.path())
            .with_database_connection(db.clone())
            .build()
            .await
            .expect("build runtime");
        let backend = Backend::new(runtime, Arc::new(db), temp.path().to_path_buf());

        let loaded = backend
            .plugin_statuses()
            .into_iter()
            .map(|status| status.plugin_id)
            .collect::<BTreeSet<_>>();

        let required = BTreeSet::from([
            tool::skills_plugin_id().to_string(),
            tool::lsp_plugin_id().to_string(),
            tool::cron_plugin_id().to_string(),
            tool::fs_plugin_id().to_string(),
            tool::settings_plugin_id().to_string(),
            tool::shell_plugin_id().to_string(),
            tool::web_plugin_id().to_string(),
            tool::workflow_plugin_id().to_string(),
            memory::memory_plugin_id().to_string(),
        ]);

        for plugin_id in required {
            assert!(
                loaded.contains(plugin_id.as_str()),
                "missing builtin runtime plugin {plugin_id}; loaded={loaded:?}"
            );
        }
    }
}
