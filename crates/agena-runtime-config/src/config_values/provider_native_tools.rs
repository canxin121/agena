use super::{BTreeMap, Deserialize, Serialize};
use agena_provider::{ProviderNativeToolHarnessKind, ProviderNativeToolHarnessRef};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Viewport size for a browser harness.
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
/// Configuration of a browser harness.
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
/// Configuration of a shell harness.
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
/// Configuration of an editor harness.
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
/// Named harness configurations by kind.
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

    pub fn contains(&self, reference: &ProviderNativeToolHarnessRef) -> bool {
        match reference.kind {
            ProviderNativeToolHarnessKind::Browser => {
                self.browser.contains_key(reference.name.as_str())
            }
            ProviderNativeToolHarnessKind::Shell => {
                self.shell.contains_key(reference.name.as_str())
            }
            ProviderNativeToolHarnessKind::Editor => {
                self.editor.contains_key(reference.name.as_str())
            }
        }
    }
}
