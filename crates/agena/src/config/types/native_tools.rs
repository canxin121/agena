use super::{BTreeMap, Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNativeToolKind {
    WebSearch,
    FileSearch,
    CodeExecution,
    ImageGeneration,
    Computer,
    Bash,
    TextEditor,
    UrlContext,
    RemoteMcp,
}

impl ProviderNativeToolKind {
    pub const ALL: [Self; 9] = [
        Self::WebSearch,
        Self::FileSearch,
        Self::CodeExecution,
        Self::ImageGeneration,
        Self::Computer,
        Self::Bash,
        Self::TextEditor,
        Self::UrlContext,
        Self::RemoteMcp,
    ];

    pub const fn config_key(self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
            Self::FileSearch => "file_search",
            Self::CodeExecution => "code_execution",
            Self::ImageGeneration => "image_generation",
            Self::Computer => "computer",
            Self::Bash => "bash",
            Self::TextEditor => "text_editor",
            Self::UrlContext => "url_context",
            Self::RemoteMcp => "remote_mcp",
        }
    }

    pub const fn supports_route(self, route: ProviderNativeToolRoute) -> bool {
        match self {
            Self::WebSearch => matches!(
                route,
                ProviderNativeToolRoute::Disabled
                    | ProviderNativeToolRoute::Plugin
                    | ProviderNativeToolRoute::ProviderHosted
            ),
            Self::FileSearch | Self::CodeExecution | Self::ImageGeneration | Self::UrlContext => {
                matches!(
                    route,
                    ProviderNativeToolRoute::Disabled | ProviderNativeToolRoute::ProviderHosted
                )
            }
            Self::Computer | Self::Bash | Self::TextEditor => matches!(
                route,
                ProviderNativeToolRoute::Disabled | ProviderNativeToolRoute::ProviderHarness
            ),
            Self::RemoteMcp => matches!(
                route,
                ProviderNativeToolRoute::Disabled | ProviderNativeToolRoute::ProviderConnector
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNativeToolRoute {
    Disabled,
    Plugin,
    ProviderHosted,
    ProviderHarness,
    ProviderConnector,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NativeToolUserLocationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl NativeToolUserLocationConfig {
    pub const fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.region.is_none()
            && self.city.is_none()
            && self.timezone.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeToolFreshness {
    Auto,
    Cached,
    Live,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedWebSearchConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_domains: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness: Option<NativeToolFreshness>,
    #[serde(
        default,
        skip_serializing_if = "NativeToolUserLocationConfig::is_empty"
    )]
    pub user_location: NativeToolUserLocationConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedWebSearchConfig {
    pub fn is_empty(&self) -> bool {
        self.allowed_domains.is_empty()
            && self.blocked_domains.is_empty()
            && self.freshness.is_none()
            && self.user_location.is_empty()
            && self.max_results.is_none()
            && self.search_context_size.is_none()
            && self.provider_options.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedFileSearchConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vector_store_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_results: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedFileSearchConfig {
    pub fn is_empty(&self) -> bool {
        self.vector_store_ids.is_empty()
            && self.max_results.is_none()
            && self.include_results.is_none()
            && self.provider_options.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HostedCodeExecutionContainerConfig {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_ids: Vec<String>,
}

impl HostedCodeExecutionContainerConfig {
    pub fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.id.is_none()
            && self.memory_limit.is_none()
            && self.file_ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedCodeExecutionConfig {
    #[serde(
        default,
        skip_serializing_if = "HostedCodeExecutionContainerConfig::is_empty"
    )]
    pub container: HostedCodeExecutionContainerConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedCodeExecutionConfig {
    pub fn is_empty(&self) -> bool {
        self.container.is_empty() && self.provider_options.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedImageGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedImageGenerationConfig {
    pub fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.size.is_none()
            && self.quality.is_none()
            && self.moderation.is_none()
            && self.provider_options.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedUrlContextConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_urls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedUrlContextConfig {
    pub fn is_empty(&self) -> bool {
        self.max_urls.is_none() && self.provider_options.is_none()
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeToolHarnessKind {
    Browser,
    Shell,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderNativeHarnessRef {
    pub kind: NativeToolHarnessKind,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeHarnessBindings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<ProviderNativeHarnessRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bash: Option<ProviderNativeHarnessRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_editor: Option<ProviderNativeHarnessRef>,
}

impl ProviderNativeHarnessBindings {
    pub const fn is_empty(&self) -> bool {
        self.computer.is_none() && self.bash.is_none() && self.text_editor.is_none()
    }

    pub fn binding_for(&self, tool: ProviderNativeToolKind) -> Option<&ProviderNativeHarnessRef> {
        match tool {
            ProviderNativeToolKind::Computer => self.computer.as_ref(),
            ProviderNativeToolKind::Bash => self.bash.as_ref(),
            ProviderNativeToolKind::TextEditor => self.text_editor.as_ref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeConnectorConfig {
    pub server: String,
    pub require_approval: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_filter: Vec<String>,
}

impl Default for ProviderNativeConnectorConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            require_approval: true,
            tool_filter: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolsConfig {
    pub enabled: bool,
    #[serde(
        default,
        skip_serializing_if = "ProviderNativeToolRoutesConfig::is_empty"
    )]
    pub routes: ProviderNativeToolRoutesConfig,
    #[serde(default, skip_serializing_if = "ProviderHostedToolConfigs::is_empty")]
    pub hosted: ProviderHostedToolConfigs,
    #[serde(
        default,
        skip_serializing_if = "ProviderNativeHarnessBindings::is_empty"
    )]
    pub harness: ProviderNativeHarnessBindings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub connectors: BTreeMap<String, ProviderNativeConnectorConfig>,
}

impl ProviderNativeToolsConfig {
    pub fn is_empty(&self) -> bool {
        !self.enabled
            && self.routes.is_empty()
            && self.hosted.is_empty()
            && self.harness.is_empty()
            && self.connectors.is_empty()
    }

    pub fn bindings(&self) -> Vec<ProviderNativeToolBinding> {
        if !self.enabled {
            return Vec::new();
        }

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderNativeToolBinding {
    pub tool: ProviderNativeToolKind,
    pub route: ProviderNativeToolRoute,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<ProviderNativeHarnessRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connector_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HarnessViewportConfig {
    pub width: u32,
    pub height: u32,
}

impl HarnessViewportConfig {
    pub const fn is_empty(&self) -> bool {
        self.width == 0 && self.height == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserHarnessConfig {
    pub driver: String,
    pub headless: bool,
    #[serde(default, skip_serializing_if = "HarnessViewportConfig::is_empty")]
    pub viewport: HarnessViewportConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_options: Option<serde_json::Value>,
}

impl Default for BrowserHarnessConfig {
    fn default() -> Self {
        Self {
            driver: "playwright".to_owned(),
            headless: true,
            viewport: HarnessViewportConfig::default(),
            allowed_domains: Vec::new(),
            launch_options: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellHarnessConfig {
    pub workspace_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl Default for ShellHarnessConfig {
    fn default() -> Self {
        Self {
            workspace_only: true,
            allow_commands: Vec::new(),
            deny_commands: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorHarnessConfig {
    pub workspace_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_file_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_extensions: Vec<String>,
}

impl Default for EditorHarnessConfig {
    fn default() -> Self {
        Self {
            workspace_only: true,
            max_file_bytes: None,
            allowed_extensions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HarnessesConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub browser: BTreeMap<String, BrowserHarnessConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shell: BTreeMap<String, ShellHarnessConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub editor: BTreeMap<String, EditorHarnessConfig>,
}

impl HarnessesConfig {
    pub fn is_empty(&self) -> bool {
        self.browser.is_empty() && self.shell.is_empty() && self.editor.is_empty()
    }

    pub fn contains(&self, reference: &ProviderNativeHarnessRef) -> bool {
        match reference.kind {
            NativeToolHarnessKind::Browser => self.browser.contains_key(reference.name.as_str()),
            NativeToolHarnessKind::Shell => self.shell.contains_key(reference.name.as_str()),
            NativeToolHarnessKind::Editor => self.editor.contains_key(reference.name.as_str()),
        }
    }
}
