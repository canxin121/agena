use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Descriptor of a provider contributed by a plugin.
pub struct ProviderDescriptor {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub kind: ProviderKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Protocol kind of a contributed provider.
pub enum ProviderKind {
    #[default]
    Custom,
    OpenAiCompatible,
    AnthropicCompatible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Input of the provider list hook.
pub struct ProviderListInput {
    pub current: Vec<ProviderDescriptor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Patch applied to the provider list by a hook.
pub struct ProviderListPatch {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add: Vec<ProviderDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove: Vec<String>,
}
