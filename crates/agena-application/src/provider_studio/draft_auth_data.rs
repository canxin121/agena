//! Data types for provider draft authentication and provider-studio save
//! results, migrated from `agena-tui-backend/src/backend_drafts/provider_draft_auth.rs`.
//!
//! The interactive auth flow entry points (`start_provider_draft_auth` /
//! `continue_provider_draft_auth`) live on [`crate::Application`], while the
//! transport-safe draft and result types they operate on live here.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::catalog::credential_issuer_label;
use super::draft_config::ProviderConfigDraft;
use super::save::parse_credential_issuer;
use agena_provider::CredentialIssuer;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Kind of a draft provider authentication flow.
pub enum ProviderDraftAuthKind {
    Unset,
    None,
    ApiPending,
    Api,
    ClineApi,
    Gitlab,
    Credential(Option<CredentialIssuer>),
    BedrockSigv4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Adapter rule of a draft provider configuration.
pub struct ProviderDraftAdapterRule {
    pub adapter_id: &'static str,
    pub detail_key: &'static str,
    pub requires_base_url: bool,
    pub supports_draft_model_listing: bool,
}

pub(crate) const NONE_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[ProviderDraftAdapterRule {
    adapter_id: "ollama",
    detail_key: "provider-adapter-rule-none-ollama-detail",
    requires_base_url: false,
    supports_draft_model_listing: true,
}];

pub(crate) const API_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai_responses",
        detail_key: "provider-adapter-rule-api-openai-detail",
        requires_base_url: true,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "openai_chat_completions",
        detail_key: "provider-adapter-rule-api-openai-detail",
        requires_base_url: true,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "openai_realtime",
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

pub(crate) const CLINE_API_ADAPTER_RULES: &[ProviderDraftAdapterRule] =
    &[ProviderDraftAdapterRule {
        adapter_id: "openai_chat_completions",
        detail_key: "provider-adapter-rule-cline-api-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    }];

pub(crate) const GITLAB_AUTH_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai_responses",
        detail_key: "provider-adapter-rule-gitlab-auth-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "openai_chat_completions",
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

pub(crate) const OPENAI_CHATGPT_ADAPTER_RULES: &[ProviderDraftAdapterRule] =
    &[ProviderDraftAdapterRule {
        adapter_id: "openai_responses",
        detail_key: "provider-adapter-rule-openai-chatgpt-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    }];

pub(crate) const GITHUB_COPILOT_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai_responses",
        detail_key: "provider-adapter-rule-github-copilot-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "openai_chat_completions",
        detail_key: "provider-adapter-rule-github-copilot-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail_key: "provider-adapter-rule-github-copilot-anthropic-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
];

pub(crate) const GITLAB_CREDENTIAL_ADAPTER_RULES: &[ProviderDraftAdapterRule] = &[
    ProviderDraftAdapterRule {
        adapter_id: "openai_responses",
        detail_key: "provider-adapter-rule-gitlab-credential-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "openai_chat_completions",
        detail_key: "provider-adapter-rule-gitlab-credential-openai-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
    ProviderDraftAdapterRule {
        adapter_id: "anthropic",
        detail_key: "provider-adapter-rule-gitlab-credential-anthropic-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    },
];

pub(crate) const GOOGLE_ADC_ADAPTER_RULES: &[ProviderDraftAdapterRule] =
    &[ProviderDraftAdapterRule {
        adapter_id: "openai_chat_completions",
        detail_key: "provider-adapter-rule-google-adc-openai-detail",
        requires_base_url: true,
        supports_draft_model_listing: true,
    }];

pub(crate) const SAP_AI_CORE_ADAPTER_RULES: &[ProviderDraftAdapterRule] =
    &[ProviderDraftAdapterRule {
        adapter_id: "openai_chat_completions",
        detail_key: "provider-adapter-rule-sap-ai-core-openai-detail",
        requires_base_url: true,
        supports_draft_model_listing: true,
    }];

pub(crate) const BEDROCK_SIGV4_ADAPTER_RULES: &[ProviderDraftAdapterRule] =
    &[ProviderDraftAdapterRule {
        adapter_id: "amazon_bedrock",
        detail_key: "provider-adapter-rule-bedrock-sigv4-amazon-bedrock-detail",
        requires_base_url: false,
        supports_draft_model_listing: true,
    }];

pub(crate) const DEFAULT_LOCAL_OAUTH_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub(crate) const LEGACY_LOCAL_OAUTH_REDIRECT_URIS: &[&str] = &[
    "http://127.0.0.1:1455/callback",
    "http://127.0.0.1:1455/auth/callback",
];
pub(crate) const DEFAULT_GITLAB_INSTANCE_URL: &str = "https://gitlab.com";
pub(crate) const CLINE_API_MODELS_URL: &str =
    "https://api.cline.bot/api/v1/ai/cline/recommended-models";

impl ProviderDraftAuthKind {
    pub fn label(&self) -> String {
        match self {
            Self::Unset => "unset".to_owned(),
            Self::None => "none".to_owned(),
            Self::ApiPending => "api".to_owned(),
            Self::Api => "api".to_owned(),
            Self::ClineApi => "cline_api".to_owned(),
            Self::Gitlab => "gitlab_api".to_owned(),
            Self::Credential(Some(issuer)) => {
                format!("credential:{}", credential_issuer_label(*issuer))
            }
            Self::Credential(None) => "credential".to_owned(),
            Self::BedrockSigv4 => "bedrock_sigv4".to_owned(),
        }
    }

    pub fn supports_draft_model_listing(&self) -> bool {
        self.adapter_rules()
            .iter()
            .any(|rule| rule.supports_draft_model_listing)
    }

    pub fn mode_label(&self) -> &'static str {
        match self {
            Self::Unset => "",
            Self::None => "none",
            Self::ApiPending | Self::Api | Self::ClineApi | Self::Gitlab | Self::BedrockSigv4 => {
                "api"
            }
            Self::Credential(_) => "credential",
        }
    }

    pub fn subtype_label(&self) -> &'static str {
        match self {
            Self::Unset | Self::None | Self::ApiPending | Self::Credential(None) => "",
            Self::Api => "custom",
            Self::ClineApi => "cline_api",
            Self::Gitlab => "gitlab_api",
            Self::Credential(Some(issuer)) => credential_issuer_label(*issuer),
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
            Self::ApiPending => &[],
            Self::Api => API_ADAPTER_RULES,
            Self::ClineApi => CLINE_API_ADAPTER_RULES,
            Self::Gitlab => GITLAB_AUTH_ADAPTER_RULES,
            Self::Credential(None) => &[],
            Self::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => OPENAI_CHATGPT_ADAPTER_RULES,
            Self::Credential(Some(CredentialIssuer::GithubCopilot)) => GITHUB_COPILOT_ADAPTER_RULES,
            Self::Credential(Some(CredentialIssuer::Gitlab)) => GITLAB_CREDENTIAL_ADAPTER_RULES,
            Self::Credential(Some(CredentialIssuer::GoogleAdc)) => GOOGLE_ADC_ADAPTER_RULES,
            Self::Credential(Some(CredentialIssuer::SapAiCore)) => SAP_AI_CORE_ADAPTER_RULES,
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

    pub fn parse_category(value: &str, current: Self) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "" => Ok(Self::Unset),
            "none" => Ok(Self::None),
            "api" => Ok(match current {
                Self::ClineApi => Self::ClineApi,
                Self::Gitlab => Self::Gitlab,
                Self::BedrockSigv4 => Self::BedrockSigv4,
                Self::Api => Self::Api,
                Self::ApiPending | Self::Unset | Self::None | Self::Credential(_) => {
                    Self::ApiPending
                }
            }),
            "credential" => Ok(match current {
                Self::Credential(Some(issuer)) => Self::Credential(Some(issuer)),
                Self::Credential(None)
                | Self::Unset
                | Self::None
                | Self::ApiPending
                | Self::Api
                | Self::ClineApi
                | Self::Gitlab
                | Self::BedrockSigv4 => Self::Credential(None),
            }),
            _ => Err(anyhow!(
                "unsupported auth_mode `{}`; expected none, api, or credential",
                value.trim()
            )),
        }
    }

    pub fn parse_subtype(value: &str, current: Self) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match current {
            Self::ApiPending | Self::Api | Self::ClineApi | Self::Gitlab | Self::BedrockSigv4 => {
                match normalized.as_str() {
                    "" => Ok(Self::ApiPending),
                    "custom" => Ok(Self::Api),
                    "cline_api" => Ok(Self::ClineApi),
                    "gitlab_api" => Ok(Self::Gitlab),
                    "bedrock_sigv4" => Ok(Self::BedrockSigv4),
                    _ => Err(anyhow!(
                        "unsupported api auth subtype `{}`; expected custom, cline_api, gitlab_api, or bedrock_sigv4",
                        value.trim()
                    )),
                }
            }
            Self::Credential(_) => {
                if normalized.is_empty() {
                    Ok(Self::Credential(None))
                } else {
                    parse_credential_issuer(normalized.as_str())
                        .map(|issuer| Self::Credential(Some(issuer)))
                }
            }
            Self::Unset | Self::None => Err(anyhow!(
                "auth subtype is only available after selecting api or credential auth"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
/// Secret source kind of a draft provider.
pub enum ProviderDraftSecretSourceKind {
    #[default]
    Unset,
    Inline,
    Env,
}

impl ProviderDraftSecretSourceKind {
    pub fn token(self) -> &'static str {
        match self {
            Self::Unset => "",
            Self::Inline => "inline",
            Self::Env => "env",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" => Ok(Self::Unset),
            "inline" => Ok(Self::Inline),
            "env" => Ok(Self::Env),
            other => Err(anyhow!(
                "unsupported secret source `{other}`; expected inline or env"
            )),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Draft OAuth tokens of a provider.
pub struct ProviderOAuthTokensDraft {
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at_ms: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Draft browser auth session of a provider.
pub struct ProviderBrowserAuthSessionDraft {
    pub authorize_url: String,
    pub display_url: Option<String>,
    pub state: String,
    pub pkce_verifier: String,
}

impl ProviderBrowserAuthSessionDraft {
    pub fn display_authorize_url(&self) -> &str {
        self.display_url
            .as_deref()
            .unwrap_or(self.authorize_url.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Draft device auth session of a provider.
pub struct ProviderDeviceAuthSessionDraft {
    pub verification_url: String,
    pub display_url: Option<String>,
    pub user_code: String,
    pub device_code: String,
    pub interval_seconds: u64,
}

impl ProviderDeviceAuthSessionDraft {
    pub fn display_verification_url(&self) -> &str {
        self.display_url
            .as_deref()
            .unwrap_or(self.verification_url.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
/// Kind of interactive login in a draft auth flow.
pub enum ProviderDraftInteractiveLoginKind {
    Browser,
    #[default]
    Device,
}

impl ProviderDraftInteractiveLoginKind {
    pub fn token(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Device => "device",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "browser" => Some(Self::Browser),
            "device" => Some(Self::Device),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Draft credential for the OpenAI ChatGPT provider.
pub struct OpenAiChatgptCredentialDraft {
    pub login_kind: ProviderDraftInteractiveLoginKind,
    pub redirect_uri: String,
    pub callback_url: String,
    pub tokens: ProviderOAuthTokensDraft,
    pub account_id: String,
    pub browser: Option<ProviderBrowserAuthSessionDraft>,
    pub device: Option<ProviderDeviceAuthSessionDraft>,
}

impl OpenAiChatgptCredentialDraft {
    /// Clear any in-progress interactive authentication state.
    pub fn clear_pending(&mut self) {
        self.callback_url.clear();
        self.browser = None;
        self.device = None;
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Draft credential for the GitHub Copilot provider.
pub struct GithubCopilotCredentialDraft {
    pub enterprise_domain: String,
    pub tokens: ProviderOAuthTokensDraft,
    pub device: Option<ProviderDeviceAuthSessionDraft>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Draft credential for the GitLab provider.
pub struct GitlabCredentialDraft {
    pub redirect_uri: String,
    pub callback_url: String,
    pub tokens: ProviderOAuthTokensDraft,
    pub browser: Option<ProviderBrowserAuthSessionDraft>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Bundle of draft credentials for a provider.
pub struct ProviderCredentialDraftBundle {
    pub openai_chatgpt: OpenAiChatgptCredentialDraft,
    pub github_copilot: GithubCopilotCredentialDraft,
    pub gitlab: GitlabCredentialDraft,
}

impl ProviderCredentialDraftBundle {
    pub(crate) fn normalize_shape(&mut self) {
        if self.openai_chatgpt.redirect_uri.trim().is_empty()
            || LEGACY_LOCAL_OAUTH_REDIRECT_URIS
                .iter()
                .any(|legacy| self.openai_chatgpt.redirect_uri.trim() == *legacy)
        {
            self.openai_chatgpt.redirect_uri = DEFAULT_LOCAL_OAUTH_REDIRECT_URI.to_owned();
        }
        if self.gitlab.redirect_uri.trim().is_empty() {
            self.gitlab.redirect_uri = DEFAULT_LOCAL_OAUTH_REDIRECT_URI.to_owned();
        }
    }

    pub(crate) fn active_tokens(
        &self,
        issuer: Option<CredentialIssuer>,
    ) -> Option<&ProviderOAuthTokensDraft> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => Some(&self.openai_chatgpt.tokens),
            Some(CredentialIssuer::GithubCopilot) => Some(&self.github_copilot.tokens),
            Some(CredentialIssuer::Gitlab) => Some(&self.gitlab.tokens),
            Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => None,
        }
    }

    pub(crate) fn active_tokens_mut(
        &mut self,
        issuer: Option<CredentialIssuer>,
    ) -> Option<&mut ProviderOAuthTokensDraft> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => Some(&mut self.openai_chatgpt.tokens),
            Some(CredentialIssuer::GithubCopilot) => Some(&mut self.github_copilot.tokens),
            Some(CredentialIssuer::Gitlab) => Some(&mut self.gitlab.tokens),
            Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => None,
        }
    }

    pub(crate) fn redirect_uri(&self, issuer: Option<CredentialIssuer>) -> Option<&str> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => {
                Some(self.openai_chatgpt.redirect_uri.as_str())
            }
            Some(CredentialIssuer::Gitlab) => Some(self.gitlab.redirect_uri.as_str()),
            _ => None,
        }
    }

    pub(crate) fn callback_url(&self, issuer: Option<CredentialIssuer>) -> Option<&str> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => {
                Some(self.openai_chatgpt.callback_url.as_str())
            }
            Some(CredentialIssuer::Gitlab) => Some(self.gitlab.callback_url.as_str()),
            _ => None,
        }
    }

    pub(crate) fn account_id(&self, issuer: Option<CredentialIssuer>) -> Option<&str> {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => Some(self.openai_chatgpt.account_id.as_str()),
            _ => None,
        }
    }

    pub(crate) fn set_redirect_uri(&mut self, issuer: Option<CredentialIssuer>, value: String) {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => self.openai_chatgpt.redirect_uri = value,
            Some(CredentialIssuer::Gitlab) => self.gitlab.redirect_uri = value,
            _ => {}
        }
    }

    pub(crate) fn set_callback_url(&mut self, issuer: Option<CredentialIssuer>, value: String) {
        match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => self.openai_chatgpt.callback_url = value,
            Some(CredentialIssuer::Gitlab) => self.gitlab.callback_url = value,
            _ => {}
        }
    }

    pub(crate) fn set_account_id(&mut self, issuer: Option<CredentialIssuer>, value: String) {
        if let Some(CredentialIssuer::OpenaiChatgpt) = issuer {
            self.openai_chatgpt.account_id = value;
        }
    }
}

/// Build draft credential bundle from the configured provider's OAuth data.
pub(crate) fn provider_credential_drafts(
    issuer: CredentialIssuer,
    credential: Option<&AuthData>,
) -> ProviderCredentialDraftBundle {
    let Some(AuthData::OAuth {
        refresh,
        access,
        expires_at_ms,
        account_id,
        enterprise_url,
        ..
    }) = credential
    else {
        return ProviderCredentialDraftBundle::default();
    };

    let tokens = ProviderOAuthTokensDraft {
        refresh_token: refresh.clone(),
        access_token: access.clone(),
        expires_at_ms: (*expires_at_ms).to_string(),
    };
    match issuer {
        CredentialIssuer::OpenaiChatgpt => ProviderCredentialDraftBundle {
            openai_chatgpt: OpenAiChatgptCredentialDraft {
                tokens,
                account_id: account_id.clone().unwrap_or_default(),
                ..OpenAiChatgptCredentialDraft::default()
            },
            github_copilot: GithubCopilotCredentialDraft::default(),
            gitlab: GitlabCredentialDraft::default(),
        },
        CredentialIssuer::GithubCopilot => ProviderCredentialDraftBundle {
            openai_chatgpt: OpenAiChatgptCredentialDraft::default(),
            github_copilot: GithubCopilotCredentialDraft {
                enterprise_domain: enterprise_url.clone().unwrap_or_default(),
                tokens,
                device: None,
            },
            gitlab: GitlabCredentialDraft::default(),
        },
        CredentialIssuer::Gitlab => ProviderCredentialDraftBundle {
            openai_chatgpt: OpenAiChatgptCredentialDraft::default(),
            github_copilot: GithubCopilotCredentialDraft::default(),
            gitlab: GitlabCredentialDraft {
                tokens,
                ..GitlabCredentialDraft::default()
            },
        },
        CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore => {
            ProviderCredentialDraftBundle::default()
        }
    }
}

use agena_provider::AuthData;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Message sent to the draft auth flow.
pub enum ProviderDraftAuthMessage {
    OpenaiBrowserStarted,
    OpenaiDeviceStarted { user_code: String },
    CopilotDeviceStarted { user_code: String },
    GitlabBrowserStarted,
    OpenaiPending,
    OpenaiCredentialCaptured,
    CopilotPending,
    CopilotCredentialCaptured,
    GitlabCredentialCaptured,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Field of the draft auth form.
pub enum ProviderDraftAuthField {
    RedirectUri,
    InstanceUrl,
    CallbackUrl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Error of the draft auth flow.
pub enum ProviderDraftAuthError {
    UnsupportedInteractiveLogin,
    StartBrowserAuthFirst,
    StartDeviceAuthFirst,
    RequiredField(ProviderDraftAuthField),
    Other(agena_failure::UserProblem),
}

impl ProviderDraftAuthError {
    /// Project an authentication backend diagnostic into a safe user problem.
    pub fn other(error: impl std::fmt::Display) -> Self {
        Self::Other(provider_backend_problem(
            "provider.authentication_failed",
            "Provider authentication could not be completed.",
            error,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Result of a draft auth action.
pub struct ProviderDraftAuthActionResult {
    pub draft: ProviderConfigDraft,
    pub message: ProviderDraftAuthMessage,
    pub clipboard_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Result of saving a provider studio draft.
pub enum ProviderStudioSaveResult {
    ProviderDraftSaved {
        provider_id: String,
        default_adapter: String,
        default_model: Option<String>,
    },
    AdapterMatchesSaved {
        provider_id: String,
        adapter_id: String,
        listed_model_count: usize,
        matched_model_count: usize,
    },
    ConfiguredModelSaved {
        provider_id: String,
        adapter_id: String,
        model_id: String,
    },
    ProviderDeleted {
        provider_id: String,
    },
    AdapterDeleted {
        provider_id: String,
        adapter_id: String,
        removed_model_count: usize,
    },
    ModelDeleted {
        provider_id: String,
        adapter_id: String,
        model_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Field that failed to save in the provider studio.
pub enum ProviderStudioSaveField {
    ProviderId,
    DefaultAdapter,
    AdapterId,
    ModelId,
    AuthMode,
    AuthSubtype,
    CredentialIssuer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Validation error when saving a provider studio draft.
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Error when saving a provider studio draft.
pub enum ProviderStudioSaveError {
    Validation(ProviderStudioSaveValidationError),
    ExistingProviderSettingsMustBeObject,
    ProviderAdapterMustBeObject { adapter_id: String },
    ProviderModelConfigMustBeObject,
    ConfiguredProviderAdapterSettingsMustBeObject,
    ConfiguredProviderAdapterModelsMustBeObject,
    Other(agena_failure::UserProblem),
}

impl ProviderStudioSaveError {
    pub(crate) fn other(error: impl std::fmt::Display) -> Self {
        Self::Other(provider_backend_problem(
            "provider.settings_save_failed",
            "The provider settings could not be saved.",
            error,
        ))
    }
}

impl From<agena_runtime::RuntimeConfigSettingsError> for ProviderStudioSaveError {
    fn from(error: agena_runtime::RuntimeConfigSettingsError) -> Self {
        // Preserve the structured failure: config validation errors already
        // project a safe user-visible message (and never a Reference), while
        // unexpected failures keep their reference id. Wrapping the error in a
        // generic internal problem would hide the actionable validation message
        // behind "The provider settings could not be saved."
        let problem = error.failure().into();
        Self::Other(problem)
    }
}

fn provider_backend_problem(
    code: &'static str,
    fallback: &'static str,
    _diagnostic: impl std::fmt::Display,
) -> agena_failure::UserProblem {
    // The original TUI backend logged the diagnostic via `tracing::error!`.
    // agena-application does not depend on `tracing`, so the structured failure
    // is projected without the diagnostic log line; the user-facing message and
    // reference id are unchanged.
    let failure = agena_failure::Failure::new(
        agena_failure::FailureCode::new(code),
        agena_failure::FailureCategory::Internal,
        agena_failure::FailureResponsibility::System,
        agena_failure::RetryDirective::Unknown,
        agena_failure::RecoveryDirective::Retry,
        agena_failure::FailureImpact::RequestRejected,
        agena_failure::UserPresentation::new(code, fallback),
    );
    failure.into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Details of a provider draft authentication.
pub struct ProviderDraftAuthDetails {
    pub base_url: String,
    pub instance_url: String,
    pub secret_source_kind: ProviderDraftSecretSourceKind,
    pub secret_source_value: String,
    pub credential_issuer: String,
    pub region: String,
    pub profile: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub service_key_env: String,
}
