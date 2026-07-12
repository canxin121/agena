use std::fmt;

use super::{
    AuthData, BTreeMap, CapabilityFamily, ConfigError, ConfiguredModelDefinition, CredentialIssuer,
    Deserialize, FromStr, GeminiStreamMode, OpenAiApiMode, OpenAiBackend, OpenAiStreamMode,
    ProviderNativeToolBinding, ProviderNativeToolsConfig, Serialize, ValueEnum,
};

pub type ProviderDefaultsConfig = crate::agents::AgentSelectionConfig;

pub const CLINE_API_BASE_URL: &str = "https://api.cline.bot";
pub const CLINE_API_OPENAI_PROTOCOL_PATH: &str = "/api/v1";

pub fn cline_api_protocol_paths() -> ProviderProtocolPathsConfig {
    ProviderProtocolPathsConfig {
        openai: CLINE_API_OPENAI_PROTOCOL_PATH.to_owned(),
        ..ProviderProtocolPathsConfig::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ProviderAuthConfig {
    None,
    Api(ProviderApiAuthConfig),
    Credential(ProviderCredentialAuthConfig),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderProtocolPathsConfig {
    pub openai: String,
    pub anthropic: String,
    pub gemini: String,
}

impl Default for ProviderProtocolPathsConfig {
    fn default() -> Self {
        Self {
            openai: "/v1".to_owned(),
            anthropic: "/v1".to_owned(),
            gemini: "/v1beta".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderModelDiscoveryConfig {
    #[default]
    Live,
    ConfiguredOnly,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ProviderSecretSourceConfig {
    Inline(String),
    Env(String),
}

impl ProviderSecretSourceConfig {
    pub fn inline(&self) -> Option<&str> {
        match self {
            Self::Inline(value) => Some(value.as_str()),
            Self::Env(_) => None,
        }
    }

    pub fn env(&self) -> Option<&str> {
        match self {
            Self::Inline(_) => None,
            Self::Env(value) => Some(value.as_str()),
        }
    }
}

impl fmt::Debug for ProviderSecretSourceConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inline(value) => f
                .debug_tuple("ProviderSecretSourceConfig::Inline")
                .field(&redacted(Some(value.as_str())))
                .finish(),
            Self::Env(value) => f
                .debug_tuple("ProviderSecretSourceConfig::Env")
                .field(value)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ProviderGitlabApiAccessConfig {
    ApiKey { source: ProviderSecretSourceConfig },
    Credential { credential: AuthData },
}

impl ProviderGitlabApiAccessConfig {
    pub fn api_key_source(&self) -> Option<&ProviderSecretSourceConfig> {
        match self {
            Self::ApiKey { source } => Some(source),
            Self::Credential { .. } => None,
        }
    }

    pub fn credential(&self) -> Option<&AuthData> {
        match self {
            Self::ApiKey { .. } => None,
            Self::Credential { credential } => Some(credential),
        }
    }
}

impl fmt::Debug for ProviderGitlabApiAccessConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey { source } => f
                .debug_struct("ProviderGitlabApiAccessConfig::ApiKey")
                .field("source", source)
                .finish(),
            Self::Credential { credential } => f
                .debug_struct("ProviderGitlabApiAccessConfig::Credential")
                .field("credential", &credential_debug_kind(credential))
                .finish(),
        }
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ProviderInlineCredentialAuthConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<AuthData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderHttpCredentialAuthConfig {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub protocol_paths: ProviderProtocolPathsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderSapAiCoreCredentialAuthConfig {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub protocol_paths: ProviderProtocolPathsConfig,
    pub service_key_env: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ProviderGitlabCredentialAuthConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<AuthData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_gateway_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai_gateway_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub feature_flags: BTreeMap<String, bool>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "issuer", rename_all = "snake_case")]
pub enum ProviderCredentialAuthConfig {
    OpenaiChatgpt {
        #[serde(flatten)]
        config: ProviderInlineCredentialAuthConfig,
    },
    GithubCopilot {
        #[serde(flatten)]
        config: ProviderInlineCredentialAuthConfig,
    },
    Gitlab {
        #[serde(flatten)]
        config: ProviderGitlabCredentialAuthConfig,
    },
    GoogleAdc {
        #[serde(flatten)]
        config: ProviderHttpCredentialAuthConfig,
    },
    SapAiCore {
        #[serde(flatten)]
        config: ProviderSapAiCoreCredentialAuthConfig,
    },
}

impl ProviderCredentialAuthConfig {
    pub fn issuer(&self) -> CredentialIssuer {
        match self {
            Self::OpenaiChatgpt { .. } => CredentialIssuer::OpenaiChatgpt,
            Self::GithubCopilot { .. } => CredentialIssuer::GithubCopilot,
            Self::Gitlab { .. } => CredentialIssuer::Gitlab,
            Self::GoogleAdc { .. } => CredentialIssuer::GoogleAdc,
            Self::SapAiCore { .. } => CredentialIssuer::SapAiCore,
        }
    }

    pub fn credential(&self) -> Option<&AuthData> {
        match self {
            Self::OpenaiChatgpt { config } | Self::GithubCopilot { config } => {
                config.credential.as_ref()
            }
            Self::Gitlab { config } => config.credential.as_ref(),
            Self::GoogleAdc { .. } | Self::SapAiCore { .. } => None,
        }
    }

    pub fn base_url(&self) -> Option<&str> {
        match self {
            Self::GoogleAdc { config } => Some(config.base_url.as_str()),
            Self::SapAiCore { config } => Some(config.base_url.as_str()),
            _ => None,
        }
    }

    pub fn protocol_paths(&self) -> Option<&ProviderProtocolPathsConfig> {
        match self {
            Self::GoogleAdc { config } => Some(&config.protocol_paths),
            Self::SapAiCore { config } => Some(&config.protocol_paths),
            _ => None,
        }
    }

    pub fn service_key_env(&self) -> Option<&str> {
        match self {
            Self::SapAiCore { config } => Some(config.service_key_env.as_str()),
            _ => None,
        }
    }

    pub fn gitlab(&self) -> Option<&ProviderGitlabCredentialAuthConfig> {
        match self {
            Self::Gitlab { config } => Some(config),
            _ => None,
        }
    }

    pub fn inline(&self) -> Option<&ProviderInlineCredentialAuthConfig> {
        match self {
            Self::OpenaiChatgpt { config } | Self::GithubCopilot { config } => Some(config),
            _ => None,
        }
    }
}

impl fmt::Debug for ProviderCredentialAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ProviderCredentialAuthConfig");
        debug.field("issuer", &self.issuer());
        debug.field("credential", &self.credential().map(credential_debug_kind));
        if let Some(base_url) = self.base_url() {
            debug.field("base_url", &base_url);
        }
        if let Some(protocol_paths) = self.protocol_paths() {
            debug.field("protocol_paths", protocol_paths);
        }
        if let Some(service_key_env) = self.service_key_env() {
            debug.field("service_key_env", &service_key_env);
        }
        if let Some(gitlab) = self.gitlab() {
            debug
                .field("instance_url", &gitlab.instance_url)
                .field("ai_gateway_url", &gitlab.ai_gateway_url)
                .field("ai_gateway_headers", &gitlab.ai_gateway_headers)
                .field("feature_flags", &gitlab.feature_flags);
        }
        debug.finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct BedrockSigv4AuthConfig {
    pub base_url: String,
    pub region: String,
    pub profile: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
}

impl fmt::Debug for BedrockSigv4AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BedrockSigv4AuthConfig")
            .field("base_url", &self.base_url)
            .field("region", &self.region)
            .field("profile", &self.profile)
            .field("access_key_id", &redacted(self.access_key_id.as_deref()))
            .field(
                "secret_access_key",
                &redacted(self.secret_access_key.as_deref()),
            )
            .field("session_token", &redacted(self.session_token.as_deref()))
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
    OpenAi(HttpProviderAdapterConfig<OpenAiProviderOptions>),
    Anthropic(HttpProviderAdapterConfig<AnthropicProviderOptions>),
    Gemini(HttpProviderAdapterConfig<GeminiProviderOptions>),
    Gitlab(GitlabProviderOptions),
    AmazonBedrock(AmazonBedrockProviderOptions),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolvedProviderModelConfig {
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "ProviderNativeToolsConfig::is_empty")]
    pub native_tools: ProviderNativeToolsConfig,
    #[serde(flatten)]
    pub definition: ConfiguredModelDefinition,
}

impl Default for ResolvedProviderModelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            native_tools: ProviderNativeToolsConfig::default(),
            definition: ConfiguredModelDefinition::default(),
        }
    }
}

impl ResolvedProviderModelConfig {
    pub fn native_tool_bindings(&self) -> Vec<ProviderNativeToolBinding> {
        self.native_tools.bindings()
    }
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
        Some(s) if !s.is_empty() => "***redacted***",
        _ => "<none>",
    }
}

fn credential_debug_kind(value: &AuthData) -> &'static str {
    match value {
        AuthData::Api { .. } => "api",
        AuthData::OAuth { .. } => "oauth",
        AuthData::WellKnown { .. } => "well_known",
    }
}

fn is_default<T>(value: &T) -> bool
where
    T: Default + PartialEq,
{
    value == &T::default()
}

fn is_true(value: &bool) -> bool {
    *value
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiProviderOptions {
    pub backend: OpenAiBackendConfig,
    pub api_mode: OpenAiApiModeConfig,
    #[serde(skip_serializing)]
    pub api_mode_explicit: bool,
    pub stream_mode: StreamTransportMode,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapabilityFamilyConfig {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
    Anthropic,
    Gemini,
    #[serde(rename = "bedrock")]
    Bedrock,
    Gitlab,
}

impl From<ProviderCapabilityFamilyConfig> for CapabilityFamily {
    fn from(value: ProviderCapabilityFamilyConfig) -> Self {
        match value {
            ProviderCapabilityFamilyConfig::OpenAi => CapabilityFamily::OpenAi,
            ProviderCapabilityFamilyConfig::OpenAiCompatible => CapabilityFamily::OpenAiCompatible,
            ProviderCapabilityFamilyConfig::Anthropic => CapabilityFamily::Anthropic,
            ProviderCapabilityFamilyConfig::Gemini => CapabilityFamily::Gemini,
            ProviderCapabilityFamilyConfig::Bedrock => CapabilityFamily::Bedrock,
            ProviderCapabilityFamilyConfig::Gitlab => CapabilityFamily::Gitlab,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ConfigOutputFormat {
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamTransportMode {
    Sse,
    #[serde(rename = "realtime_websocket")]
    RealtimeWebSocket,
}

impl FromStr for StreamTransportMode {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "sse" => Ok(Self::Sse),
            "realtime_websocket" => Ok(Self::RealtimeWebSocket),
            _ => Err(ConfigError::InvalidOverride(format!(
                "unknown stream mode `{value}`"
            ))),
        }
    }
}

impl From<StreamTransportMode> for OpenAiStreamMode {
    fn from(value: StreamTransportMode) -> Self {
        match value {
            StreamTransportMode::Sse => Self::Sse,
            StreamTransportMode::RealtimeWebSocket => Self::RealtimeWebSocket,
        }
    }
}

impl From<StreamTransportMode> for GeminiStreamMode {
    fn from(value: StreamTransportMode) -> Self {
        match value {
            StreamTransportMode::Sse => Self::Sse,
            StreamTransportMode::RealtimeWebSocket => Self::RealtimeWebSocket,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiApiModeConfig {
    Responses,
    Chat,
    Auto,
}

impl FromStr for OpenAiApiModeConfig {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "responses" => Ok(Self::Responses),
            "chat" => Ok(Self::Chat),
            "auto" => Ok(Self::Auto),
            _ => Err(ConfigError::InvalidOverride(format!(
                "unknown openai api mode `{value}`"
            ))),
        }
    }
}

impl From<OpenAiApiModeConfig> for OpenAiApiMode {
    fn from(value: OpenAiApiModeConfig) -> Self {
        match value {
            OpenAiApiModeConfig::Responses => Self::Responses,
            OpenAiApiModeConfig::Chat => Self::Chat,
            OpenAiApiModeConfig::Auto => Self::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiBackendConfig {
    #[default]
    Api,
    ChatgptCodex,
}

impl FromStr for OpenAiBackendConfig {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "api" => Ok(Self::Api),
            "chatgpt_codex" => Ok(Self::ChatgptCodex),
            _ => Err(ConfigError::InvalidOverride(format!(
                "unknown openai backend `{value}`"
            ))),
        }
    }
}

impl From<OpenAiBackendConfig> for OpenAiBackend {
    fn from(value: OpenAiBackendConfig) -> Self {
        match value {
            OpenAiBackendConfig::Api => Self::Api,
            OpenAiBackendConfig::ChatgptCodex => Self::ChatgptCodex,
        }
    }
}
