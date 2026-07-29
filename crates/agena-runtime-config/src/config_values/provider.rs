use std::fmt;

use super::{BTreeMap, Serialize};
use agena_provider::{
    BedrockSigv4AuthConfig, OpenAiResponsesBackendConfig, ProviderCapabilityFamilyConfig,
    ProviderCredentialAuthConfig, ProviderGitlabApiAccessConfig, ProviderModelDiscoveryConfig,
    ProviderProtocolPathsConfig, ProviderSecretSourceConfig, StreamTransportMode,
};

pub type ProviderDefaultsConfig = agena_domain::ModelSelectionConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ProviderAuthConfig {
    None,
    Api(ProviderApiAuthConfig),
    Credential(ProviderCredentialAuthConfig),
}

/// Selects whether and how tools are exposed to a provider model.
///
/// This configured value is authoritative at request time. Capability data is
/// used only when creating a new provider-model route; the runtime never
/// changes this mode or falls back to another mode implicitly.

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ProviderApiAuthConfig {
    Custom {
        #[serde(skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        #[serde(default, skip_serializing_if = "is_default")]
        protocol_paths: ProviderProtocolPathsConfig,
        #[serde(skip_serializing_if = "Option::is_none")]
        api_key: Option<ProviderSecretSourceConfig>,
    },
    #[serde(rename = "cline_api")]
    ClineApi {
        #[serde(skip_serializing_if = "Option::is_none")]
        api_key: Option<ProviderSecretSourceConfig>,
    },
    Gitlab {
        access: ProviderGitlabApiAccessConfig,
        #[serde(skip_serializing_if = "Option::is_none")]
        instance_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ai_gateway_url: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        ai_gateway_headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        feature_flags: BTreeMap<String, bool>,
    },
    BedrockSigv4 {
        base_url: String,
        region: String,
        profile: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        session_token: Option<String>,
    },
}

impl ProviderApiAuthConfig {
    pub fn custom(
        base_url: Option<String>,
        protocol_paths: ProviderProtocolPathsConfig,
        api_key: Option<ProviderSecretSourceConfig>,
    ) -> Self {
        Self::Custom {
            base_url,
            protocol_paths,
            api_key,
        }
    }

    pub fn api_key_source(&self) -> Option<&ProviderSecretSourceConfig> {
        match self {
            Self::Custom { api_key, .. } | Self::ClineApi { api_key } => api_key.as_ref(),
            Self::Gitlab { access, .. } => access.api_key_source(),
            Self::BedrockSigv4 { .. } => None,
        }
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key_source()
            .and_then(ProviderSecretSourceConfig::inline)
    }

    pub fn api_key_env(&self) -> Option<&str> {
        self.api_key_source()
            .and_then(ProviderSecretSourceConfig::env)
    }

    pub fn custom_base_url(&self) -> Option<&str> {
        match self {
            Self::Custom { base_url, .. } => base_url.as_deref(),
            _ => None,
        }
    }

    pub fn custom_protocol_paths(&self) -> Option<&ProviderProtocolPathsConfig> {
        match self {
            Self::Custom { protocol_paths, .. } => Some(protocol_paths),
            _ => None,
        }
    }

    pub fn is_cline_api(&self) -> bool {
        matches!(self, Self::ClineApi { .. })
    }

    pub fn gitlab(&self) -> Option<ProviderGitlabAuthConfig> {
        match self {
            Self::Gitlab {
                access,
                instance_url,
                ai_gateway_url,
                ai_gateway_headers,
                feature_flags,
            } => Some(ProviderGitlabAuthConfig {
                access: access.clone(),
                instance_url: instance_url.clone(),
                ai_gateway_url: ai_gateway_url.clone(),
                ai_gateway_headers: ai_gateway_headers.clone(),
                feature_flags: feature_flags.clone(),
            }),
            _ => None,
        }
    }

    pub fn bedrock_sigv4(&self) -> Option<BedrockSigv4AuthConfig> {
        match self {
            Self::BedrockSigv4 {
                base_url,
                region,
                profile,
                access_key_id,
                secret_access_key,
                session_token,
            } => Some(BedrockSigv4AuthConfig {
                base_url: base_url.clone(),
                region: region.clone(),
                profile: profile.clone(),
                access_key_id: access_key_id.clone(),
                secret_access_key: secret_access_key.clone(),
                session_token: session_token.clone(),
            }),
            _ => None,
        }
    }
}

impl Default for ProviderApiAuthConfig {
    fn default() -> Self {
        Self::Custom {
            base_url: None,
            protocol_paths: ProviderProtocolPathsConfig::default(),
            api_key: None,
        }
    }
}

impl fmt::Debug for ProviderApiAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Custom {
                base_url,
                protocol_paths,
                api_key,
            } => f
                .debug_struct("ProviderApiAuthConfig::Custom")
                .field("base_url", base_url)
                .field("protocol_paths", protocol_paths)
                .field("api_key", api_key)
                .finish(),
            Self::ClineApi { api_key } => f
                .debug_struct("ProviderApiAuthConfig::ClineApi")
                .field("api_key", api_key)
                .finish(),
            Self::Gitlab {
                access,
                instance_url,
                ai_gateway_url,
                ai_gateway_headers,
                feature_flags,
            } => f
                .debug_struct("ProviderApiAuthConfig::Gitlab")
                .field("access", access)
                .field("instance_url", instance_url)
                .field("ai_gateway_url", ai_gateway_url)
                .field("ai_gateway_headers", ai_gateway_headers)
                .field("feature_flags", feature_flags)
                .finish(),
            Self::BedrockSigv4 {
                base_url,
                region,
                profile,
                access_key_id,
                secret_access_key,
                session_token,
            } => f
                .debug_struct("ProviderApiAuthConfig::BedrockSigv4")
                .field("base_url", base_url)
                .field("region", region)
                .field("profile", profile)
                .field("access_key_id", &redacted(access_key_id.as_deref()))
                .field("secret_access_key", &redacted(secret_access_key.as_deref()))
                .field("session_token", &redacted(session_token.as_deref()))
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct ProviderGitlabAuthConfig {
    pub access: ProviderGitlabApiAccessConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_gateway_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai_gateway_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub feature_flags: BTreeMap<String, bool>,
}

impl fmt::Debug for ProviderGitlabAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderGitlabAuthConfig")
            .field("access", &self.access)
            .field("instance_url", &self.instance_url)
            .field("ai_gateway_url", &self.ai_gateway_url)
            .field("ai_gateway_headers", &self.ai_gateway_headers)
            .field("feature_flags", &self.feature_flags)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProviderAdapterConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "is_default")]
    pub model_discovery: ProviderModelDiscoveryConfig,
    #[serde(flatten)]
    pub definition: ProviderAdapterDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderAdapterDefinition {
    Ollama(OllamaProviderOptions),
    OpenAiResponses(HttpProviderAdapterConfig<OpenAiResponsesProviderOptions>),
    OpenAiChatCompletions(HttpProviderAdapterConfig<OpenAiChatCompletionsProviderOptions>),
    OpenAiRealtime(HttpProviderAdapterConfig<OpenAiRealtimeProviderOptions>),
    Anthropic(HttpProviderAdapterConfig<AnthropicProviderOptions>),
    Gemini(HttpProviderAdapterConfig<GeminiProviderOptions>),
    Gitlab(GitlabProviderOptions),
    AmazonBedrock(AmazonBedrockProviderOptions),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaProviderOptions {
    pub base_url: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct HttpProviderAdapterConfig<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub options: T,
}

impl<T: fmt::Debug> fmt::Debug for HttpProviderAdapterConfig<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpProviderAdapterConfig")
            .field("user_agent", &self.user_agent)
            .field("extra_headers", &self.extra_headers)
            .field("options", &self.options)
            .finish()
    }
}

fn redacted(value: Option<&str>) -> &'static str {
    match value {
        Some(value) if !value.is_empty() => "***redacted***",
        _ => "<none>",
    }
}

fn is_default<T>(value: &T) -> bool
where
    T: Default + PartialEq,
{
    value == &T::default()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiResponsesProviderOptions {
    pub backend: OpenAiResponsesBackendConfig,
    pub models_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_family: Option<ProviderCapabilityFamilyConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiChatCompletionsProviderOptions {
    pub models_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_family: Option<ProviderCapabilityFamilyConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiRealtimeProviderOptions {
    pub realtime_ws_url: Option<String>,
    pub models_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_family: Option<ProviderCapabilityFamilyConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnthropicProviderOptions {
    pub models_url: Option<String>,
    pub messages_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub extra_beta_header: Option<String>,
    pub eager_input_streaming: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeminiProviderOptions {
    pub auth_header: Option<String>,
    pub auth_scheme: Option<String>,
    pub stream_mode: StreamTransportMode,
    pub realtime_ws_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimpleHttpProviderOptions {
    pub auth_header: Option<String>,
    pub auth_scheme: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct GitlabProviderOptions {
    pub instance_url: Option<String>,
    pub ai_gateway_url: Option<String>,
    pub ai_gateway_headers: BTreeMap<String, String>,
    pub feature_flags: BTreeMap<String, bool>,
}

impl fmt::Debug for GitlabProviderOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitlabProviderOptions")
            .field("instance_url", &self.instance_url)
            .field("ai_gateway_url", &self.ai_gateway_url)
            .field("ai_gateway_headers", &self.ai_gateway_headers)
            .field("feature_flags", &self.feature_flags)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct AmazonBedrockProviderOptions;
