//! Settings wire values shared by HTTP and plugin callers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
/// Source layer of a settings read or write.
pub enum ConfigSettingsSource {
    #[default]
    Effective,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "SettingsLayer")]
/// Config file layer targeted by a settings edit.
pub enum ConfigSettingsLayer {
    #[default]
    Global,
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
/// Input selecting a settings path.
pub struct ConfigSettingsPathInput {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(default)]
/// Options for a settings edit operation.
pub struct ConfigSettingsEditOptions {
    /// Revision of the complete persisted JSON document, not of a leaf value.
    pub expected_revision: Option<String>,
    pub dry_run: bool,
    #[serde(default = "default_true")]
    pub validate: bool,
    #[serde(default = "default_true")]
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
/// Input for reading a settings value.
pub struct ConfigSettingsGetInput {
    #[serde(flatten)]
    pub target: ConfigSettingsPathInput,
    pub source: ConfigSettingsSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
/// Input for listing settings values.
pub struct ConfigSettingsListInput {
    #[serde(flatten)]
    pub target: ConfigSettingsPathInput,
    pub source: ConfigSettingsSource,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
/// Input for setting a settings value.
pub struct ConfigSettingsSetInput {
    pub path: String,
    pub value: JsonValue,
    #[serde(flatten)]
    pub options: ConfigSettingsEditOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
/// Input for deleting a settings value.
pub struct ConfigSettingsDeleteInput {
    pub path: String,
    #[serde(flatten)]
    pub options: ConfigSettingsEditOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
/// Input for patching settings values.
pub struct ConfigSettingsPatchInput {
    #[serde(flatten)]
    pub target: ConfigSettingsPathInput,
    pub changes: JsonValue,
    #[serde(flatten)]
    pub options: ConfigSettingsEditOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
/// Input for validating settings.
pub struct ConfigSettingsValidateInput {
    #[serde(flatten)]
    pub target: ConfigSettingsPathInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a settings read.
pub struct ConfigSettingsReadResponse {
    pub revision: Option<String>,
    pub config_path: PathBuf,
    pub config_found: bool,
    pub source: ConfigSettingsSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub value: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One listed settings item.
pub struct ConfigSettingsListItem {
    pub path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a settings listing.
pub struct ConfigSettingsListResponse {
    pub revision: Option<String>,
    pub config_path: PathBuf,
    pub config_found: bool,
    pub source: ConfigSettingsSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub items: Vec<ConfigSettingsListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a settings validation.
pub struct ConfigSettingsValidateResponse {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub valid: bool,
}

fn default_true() -> bool {
    true
}
