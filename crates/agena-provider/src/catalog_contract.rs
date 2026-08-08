use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// How a native provider tool is routed.
pub enum ProviderNativeToolRoute {
    Disabled,
    Plugin,
    ProviderHosted,
    ProviderHarness,
    ProviderConnector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Freshness policy for native provider tools.
pub enum ProviderNativeToolFreshness {
    Auto,
    Cached,
    Live,
}

/// Provider-native tool routing for one model configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolRoutesConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_search: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_execution: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_generation: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bash: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_editor: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_context: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_mcp: Option<ProviderNativeToolRoute>,
}

impl ProviderNativeToolRoutesConfig {
    pub const fn is_empty(&self) -> bool {
        self.web_search.is_none()
            && self.file_search.is_none()
            && self.code_execution.is_none()
            && self.image_generation.is_none()
            && self.computer.is_none()
            && self.bash.is_none()
            && self.text_editor.is_none()
            && self.url_context.is_none()
            && self.remote_mcp.is_none()
    }

    pub const fn route_for(&self, tool: ProviderNativeToolKind) -> Option<ProviderNativeToolRoute> {
        match tool {
            ProviderNativeToolKind::WebSearch => self.web_search,
            ProviderNativeToolKind::FileSearch => self.file_search,
            ProviderNativeToolKind::CodeExecution => self.code_execution,
            ProviderNativeToolKind::ImageGeneration => self.image_generation,
            ProviderNativeToolKind::Computer => self.computer,
            ProviderNativeToolKind::Bash => self.bash,
            ProviderNativeToolKind::TextEditor => self.text_editor,
            ProviderNativeToolKind::UrlContext => self.url_context,
            ProviderNativeToolKind::RemoteMcp => self.remote_mcp,
        }
    }
}

/// Optional geographical context passed to a provider-hosted tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolUserLocationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl ProviderNativeToolUserLocationConfig {
    pub const fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.region.is_none()
            && self.city.is_none()
            && self.timezone.is_none()
    }
}

/// A named remote connector available to a provider-native tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolConnectorConfig {
    pub server: String,
    pub require_approval: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_filter: Vec<String>,
}

impl Default for ProviderNativeToolConnectorConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            require_approval: true,
            tool_filter: Vec::new(),
        }
    }
}

/// Provider-hosted tool settings for one configured model route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedToolConfigs {
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedWebSearchConfig::is_empty"
    )]
    pub web_search: ProviderHostedWebSearchConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedFileSearchConfig::is_empty"
    )]
    pub file_search: ProviderHostedFileSearchConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedCodeExecutionConfig::is_empty"
    )]
    pub code_execution: ProviderHostedCodeExecutionConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedImageGenerationConfig::is_empty"
    )]
    pub image_generation: ProviderHostedImageGenerationConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedUrlContextConfig::is_empty"
    )]
    pub url_context: ProviderHostedUrlContextConfig,
}

impl ProviderHostedToolConfigs {
    pub fn is_empty(&self) -> bool {
        self.web_search.is_empty()
            && self.file_search.is_empty()
            && self.code_execution.is_empty()
            && self.image_generation.is_empty()
            && self.url_context.is_empty()
    }
}

/// Complete provider-native tool configuration for one model route.
/// Concrete harness settings remain runtime configuration; this value carries
/// only provider-facing routes, hosted options, stable harness references, and
/// connector declarations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolsConfig {
    #[serde(
        default,
        skip_serializing_if = "ProviderNativeToolRoutesConfig::is_empty"
    )]
    pub routes: ProviderNativeToolRoutesConfig,
    #[serde(default, skip_serializing_if = "ProviderHostedToolConfigs::is_empty")]
    pub hosted: ProviderHostedToolConfigs,
    #[serde(
        default,
        skip_serializing_if = "ProviderNativeToolHarnessBindings::is_empty"
    )]
    pub harness: ProviderNativeToolHarnessBindings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub connectors: BTreeMap<String, ProviderNativeToolConnectorConfig>,
}

impl ProviderNativeToolsConfig {
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
            && self.hosted.is_empty()
            && self.harness.is_empty()
            && self.connectors.is_empty()
    }

    pub fn bindings(&self) -> Vec<ProviderNativeToolBinding> {
        ProviderNativeToolKind::ALL
            .into_iter()
            .filter_map(|tool| {
                let route = self.routes.route_for(tool)?;
                if route == ProviderNativeToolRoute::Disabled {
                    return None;
                }
                if tool == ProviderNativeToolKind::FileSearch
                    && route == ProviderNativeToolRoute::ProviderHosted
                    && self.hosted.file_search.vector_store_ids.is_empty()
                {
                    return None;
                }
                Some(ProviderNativeToolBinding {
                    tool,
                    route,
                    harness: self.harness.binding_for(tool).cloned(),
                    connector_names: if tool == ProviderNativeToolKind::RemoteMcp {
                        self.connectors.keys().cloned().collect()
                    } else {
                        Vec::new()
                    },
                })
            })
            .collect()
    }
}

/// A resolved provider-native tool route with its stable binding references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderNativeToolBinding {
    pub tool: ProviderNativeToolKind,
    pub route: ProviderNativeToolRoute,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<ProviderNativeToolHarnessRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connector_names: Vec<String>,
}

/// Presentation-neutral defaults for one configured provider route.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderDefaults {
    pub adapter: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Summary of a configured provider adapter.
pub struct ProviderAdapterSummary {
    pub adapter_id: String,
    pub enabled: bool,
    pub configured_model_count: usize,
}

/// Complete configured adapter/model routing summary needed by presentation
/// editors. This intentionally exposes only stable route values, not Core's
/// resolved provider schema or authentication implementation details.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderConfiguredAdapterModels {
    pub adapter_id: String,
    pub enabled: bool,
    pub model_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Routing of a provider across its adapters.
pub struct ProviderConfiguredRouting {
    pub provider_id: ProviderId,
    pub adapters: Vec<ProviderConfiguredAdapterModels>,
}

/// Complete, presentation-neutral editable configuration for one saved
/// provider. The value intentionally contains stable auth/credential data
/// rather than exposing Core's resolved configuration schema to an editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfiguredEditor {
    pub provider_id: String,
    pub auth: ProviderConfiguredEditorAuth,
    pub default_adapter: Option<String>,
    pub default_model: Option<String>,
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Auth configuration for an editor provider.
pub enum ProviderConfiguredEditorAuth {
    None,
    Api {
        base_url: String,
        api_key: Option<ProviderApiKeySource>,
    },
    ClineApi {
        api_key: Option<ProviderApiKeySource>,
    },
    Gitlab {
        api_key: Option<ProviderApiKeySource>,
        instance_url: Option<String>,
    },
    Credential {
        issuer: CredentialIssuer,
        credential: Option<AuthData>,
        base_url: Option<String>,
        instance_url: Option<String>,
        service_key_env: Option<String>,
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

/// A fully projected provider entry for catalog/listing presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCatalogEntry {
    pub provider_id: ProviderId,
    pub defaults: ProviderDefaults,
    pub adapters: Vec<ProviderAdapterSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Protocol path overrides for OpenAI, Anthropic, and Gemini.
pub struct ProviderProtocolPaths {
    pub openai: String,
    pub anthropic: String,
    pub gemini: String,
}

impl Default for ProviderProtocolPaths {
    fn default() -> Self {
        Self {
            openai: "/v1".to_owned(),
            anthropic: "/v1".to_owned(),
            gemini: "/v1beta".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Where an API key comes from (inline or environment).
pub enum ProviderApiKeySource {
    Inline(String),
    Environment(String),
}

/// Stable request for live discovery against a not-yet-saved provider draft.
///
/// The request deliberately carries the draft authentication shape rather
/// than a Core configuration target, so presentation callers do not need to
/// construct or inspect the concrete configuration schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftProviderAdapterModelsRequest {
    Http(DraftHttpProviderAdapterModelsRequest),
    None {
        provider_id: Option<String>,
        adapter_ids: Vec<String>,
    },
    ClineApi {
        provider_id: Option<String>,
        api_key: Option<ProviderApiKeySource>,
        adapter_ids: Vec<String>,
        models_url: Option<String>,
    },
    Gitlab {
        provider_id: Option<String>,
        api_key: Option<ProviderApiKeySource>,
        adapter_ids: Vec<String>,
    },
    Credential {
        provider_id: Option<String>,
        issuer: CredentialIssuer,
        credential: Option<Box<AuthData>>,
        base_url: Option<String>,
        protocol_paths: ProviderProtocolPaths,
        service_key_env: Option<String>,
        instance_url: Option<String>,
        adapter_ids: Vec<String>,
    },
    BedrockSigv4 {
        provider_id: Option<String>,
        base_url: Option<String>,
        region: Option<String>,
        profile: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        session_token: Option<String>,
        adapter_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Draft request to configure an HTTP provider adapter.
pub struct DraftHttpProviderAdapterModelsRequest {
    pub provider_id: Option<String>,
    pub base_url: String,
    pub protocol_paths: ProviderProtocolPaths,
    pub api_key: Option<ProviderApiKeySource>,
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Adapter entry with resolved models and an optional failure.
pub struct ProviderAdapterModelsEntry {
    pub adapter_id: String,
    pub enabled: bool,
    pub resolved_base_url: Option<String>,
    pub models: Vec<Model>,
    pub failure: Option<agena_failure::Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Listing of adapter model entries for a provider.
pub struct ProviderAdapterModelsListing {
    pub provider_id: String,
    pub adapters: Vec<ProviderAdapterModelsEntry>,
}

/// Provider-derived values required to validate and materialize run options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModelExecutionOptions {
    pub default_adapter: Option<AdapterId>,
    pub capabilities: ModelCapabilities,
    pub thinking_modes: Vec<ModelThinkingMode>,
    pub speed_modes: BTreeMap<String, ModelSpeedMode>,
    pub metadata: ModelMetadata,
}

/// Read-only provider catalog required by application-facing provider queries.
///
/// Implementations must resolve the catalog against their current runtime
/// snapshot, so reloads are observed without rebuilding application services.
#[async_trait]
pub trait ProviderCatalog: Send + Sync {
    fn list_providers(&self) -> Vec<ProviderCatalogEntry>;

    fn contains_provider(&self, provider_id: &ProviderId) -> bool;

    fn configured_routing(&self, provider_id: &ProviderId) -> Option<ProviderConfiguredRouting>;

    fn configured_editor(&self, provider_id: &ProviderId) -> Option<ProviderConfiguredEditor>;

    /// Synchronous model choices implied by the saved configuration only.
    /// This must not perform remote provider discovery.
    fn configured_local_models(
        &self,
        provider_id: &ProviderId,
    ) -> Result<Vec<Model>, ProviderCatalogError>;

    fn default_model(&self) -> Result<Option<ModelRef>, ProviderCatalogError>;

    /// The effective default execution selection (provider/adapter/model and
    /// default thinking/speed/verbosity modes) from the merged configuration.
    /// This is the selection the runtime applies when a fresh session starts
    /// without explicit run options.
    fn default_selection(&self) -> agena_domain::ExecutionSelection;

    /// Resolve a CLI/application model target against the current configured
    /// provider catalog. `target` may be a provider or a fully qualified
    /// model target; implementations observe runtime reloads.
    fn resolve_model_target(
        &self,
        target: &str,
        model: Option<&str>,
    ) -> Result<ModelRef, ProviderCatalogError>;

    fn model_execution_options(
        &self,
        model: &ModelRef,
    ) -> Result<ProviderModelExecutionOptions, ProviderCatalogError>;

    async fn list_models(
        &self,
        provider_id: &ProviderId,
    ) -> Result<Vec<Model>, ProviderCatalogError>;

    async fn list_draft_adapter_models(
        &self,
        request: DraftProviderAdapterModelsRequest,
    ) -> Result<ProviderAdapterModelsListing, ProviderCatalogError>;

    async fn list_saved_adapter_models(
        &self,
        provider_id: &ProviderId,
        adapter_ids: Vec<String>,
    ) -> Result<ProviderAdapterModelsListing, ProviderCatalogError>;
}

/// Narrow provider-model source used while composing a model catalog.
///
/// This excludes persistence, source ranking, refresh policy, and execution
/// configuration so catalog composition can consume an adapter without
/// depending on a concrete provider registry.
#[async_trait]
pub trait ProviderModelSource: Send + Sync {
    fn provider_ids(&self) -> Vec<ProviderId>;

    async fn list_models(
        &self,
        provider_id: &ProviderId,
    ) -> Result<Vec<Model>, ProviderCatalogError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_protocol_paths_default_matches_config_default() {
        // Draft model discovery must hit the same protocol paths as a saved
        // provider config; otherwise ad-hoc listing (TUI Ctrl+R in the
        // provider studio) requests `{base_url}/models` instead of
        // `{base_url}/v1/models` and fails with 404 for OpenAI-compatible
        // gateways (e.g. cpa).
        let draft_default = ProviderProtocolPaths::default();
        let config_default = ProviderProtocolPathsConfig::default();
        assert_eq!(draft_default.openai, config_default.openai);
        assert_eq!(draft_default.anthropic, config_default.anthropic);
        assert_eq!(draft_default.gemini, config_default.gemini);
        assert_eq!(draft_default.openai, "/v1");
        assert_eq!(draft_default.gemini, "/v1beta");
    }
}
