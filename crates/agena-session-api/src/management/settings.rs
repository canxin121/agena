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
    /// Retained for wire compatibility. Permission and composed configuration
    /// validation is mandatory on public save operations, even when false.
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
    /// Whether this file layer declares the requested value. Opaque client
    /// preference nulls are local clears; optional schema nulls are unset.
    /// Omitted for effective reads and servers without layer metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_override: Option<bool>,
    /// Actual local path to clear when restoring defaults or inheritance. This
    /// is the requested leaf, an atomic schema group, or a replacing opaque
    /// ancestor that must be reset as a group. Non-atomic leaves retain the
    /// requested path's spelling; group paths use canonical quoting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_path: Option<String>,
    /// Whether editing this leaf first requires restoring a replacing local
    /// null, scalar, or array ancestor. An atomic object group can be edited at
    /// the requested leaf even when resetting it requires the whole group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_group_reset: Option<bool>,
    /// Program defaults for this settings path. These fill missing global
    /// fields and are the target of a global reset, not an editable parent layer.
    /// Omitted when unavailable; a present null means this path has no default.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_json_value"
    )]
    pub default_value: Option<JsonValue>,
    /// Resolved value through the selected file layer, before later file,
    /// environment, and command-line overrides. Uses the configuration schema's
    /// merge rules, including atomic groups and explicit preference clears.
    /// Omitted when this layer cannot be resolved; null is a known unset path.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_json_value"
    )]
    pub layer_value: Option<JsonValue>,
    /// Resolved actual parent before this file layer is applied. The global
    /// layer has no parent, so this field is omitted for global reads. Also
    /// omitted if a workspace parent cannot resolve; null is a known unset path.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_json_value"
    )]
    pub inherited_value: Option<JsonValue>,
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

fn deserialize_present_json_value<'de, D>(deserializer: D) -> Result<Option<JsonValue>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    JsonValue::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_values_round_trip_preserves_missing_null_and_falsy_values() {
        for inherited in [
            None,
            Some(JsonValue::Null),
            Some(json!(false)),
            Some(json!(0)),
            Some(json!("")),
        ] {
            let response = ConfigSettingsReadResponse {
                revision: None,
                config_path: "/fixture/config.json".into(),
                config_found: false,
                source: ConfigSettingsSource::File,
                path: Some("ui.tui.theme".to_owned()),
                value: JsonValue::Null,
                has_override: Some(false),
                override_path: None,
                requires_group_reset: Some(false),
                default_value: inherited.clone(),
                layer_value: inherited.clone(),
                inherited_value: inherited.clone(),
            };
            let mut wire = serde_json::to_value(response).unwrap();
            assert_eq!(wire.get("inherited_value"), inherited.as_ref());
            assert_eq!(wire.get("default_value"), inherited.as_ref());
            assert_eq!(wire.get("layer_value"), inherited.as_ref());
            // Presence fields remain optional for older transport responses.
            wire.as_object_mut().unwrap().remove("has_override");
            let decoded: ConfigSettingsReadResponse = serde_json::from_value(wire).unwrap();
            assert_eq!(decoded.inherited_value, inherited);
            assert_eq!(decoded.default_value, inherited);
            assert_eq!(decoded.layer_value, inherited);
            assert_eq!(decoded.has_override, None);
            assert_eq!(decoded.override_path, None);
            assert_eq!(decoded.requires_group_reset, Some(false));
        }
    }
}
