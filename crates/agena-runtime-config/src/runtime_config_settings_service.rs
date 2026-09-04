//! Stable settings-editing contracts for application transports.
//!
//! The concrete runtime remains responsible for validating a document against
//! its configuration schema and writing it to the selected configuration path.
//! These values and the transport-facing service port deliberately stay free of
//! that concrete schema so HTTP, CLI, and other presentation layers do not
//! depend on concrete Runtime configuration internals.

use std::{
    fs,
    path::{Path, PathBuf},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

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

/// Select the persisted path for a settings layer.
pub fn config_settings_layer_path<'a>(
    layer: ConfigSettingsLayer,
    global_path: &'a Path,
    workspace_path: &'a Path,
) -> &'a Path {
    match layer {
        ConfigSettingsLayer::Global => global_path,
        ConfigSettingsLayer::Workspace => workspace_path,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
/// Input selecting a settings path.
pub struct ConfigSettingsPathInput {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(default)]
/// Options for a settings edit operation.
pub struct ConfigSettingsEditOptions {
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

#[derive(Debug, Clone, Serialize)]
/// Response of a settings read.
pub struct ConfigSettingsReadResponse {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub source: ConfigSettingsSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub value: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
/// One listed settings item.
pub struct ConfigSettingsListItem {
    pub path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<JsonValue>,
}

#[derive(Debug, Clone, Serialize)]
/// Response of a settings listing.
pub struct ConfigSettingsListResponse {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub source: ConfigSettingsSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub items: Vec<ConfigSettingsListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a settings edit.
pub struct ConfigSettingsEditResponse {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub dry_run: bool,
    pub changed: bool,
    pub created: bool,
    pub deleted: bool,
    pub validated: bool,
    pub reload_requested: bool,
    pub reload_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reload: Option<ConfigSettingsReloadResponse>,
    pub previous: JsonValue,
    pub current: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a settings reload.
pub struct ConfigSettingsReloadResponse {
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: String,
}

#[derive(Debug, Clone, Serialize)]
/// Response of a settings validation.
pub struct ConfigSettingsValidateResponse {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub valid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Kind of a settings service error.
pub enum RuntimeConfigSettingsErrorKind {
    InvalidInput,
    Internal,
}

#[derive(Debug, Clone)]
/// Error returned by the settings service.
pub struct RuntimeConfigSettingsError {
    kind: RuntimeConfigSettingsErrorKind,
    failure: Box<agena_failure::Failure>,
    diagnostic: String,
}

impl RuntimeConfigSettingsError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind: RuntimeConfigSettingsErrorKind::InvalidInput,
            diagnostic: message.clone(),
            failure: Box::new(agena_failure::Failure::new(
                agena_failure::FailureCode::new("settings.invalid_input"),
                agena_failure::FailureCategory::InvalidInput,
                agena_failure::FailureResponsibility::Caller,
                agena_failure::RetryDirective::CorrectInput,
                agena_failure::RecoveryDirective::None,
                agena_failure::FailureImpact::RequestRejected,
                agena_failure::UserPresentation::validated("settings-invalid-input", &message),
            )),
        }
    }

    pub fn invalid_input_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::invalid_input(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn invalid_input_with_diagnostic(
        message: impl AsRef<str>,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        Self {
            kind: RuntimeConfigSettingsErrorKind::InvalidInput,
            diagnostic: diagnostic.to_string(),
            failure: Box::new(agena_failure::Failure::new(
                agena_failure::FailureCode::new("settings.invalid_input"),
                agena_failure::FailureCategory::InvalidInput,
                agena_failure::FailureResponsibility::Caller,
                agena_failure::RetryDirective::CorrectInput,
                agena_failure::RecoveryDirective::None,
                agena_failure::FailureImpact::RequestRejected,
                agena_failure::UserPresentation::validated("settings-invalid-input", message),
            )),
        }
    }

    pub fn internal(diagnostic: impl Into<String>) -> Self {
        let diagnostic = diagnostic.into();
        let presentation = agena_failure::UserPresentation::new(
            "settings-internal",
            "Couldn’t update the settings.",
        );
        Self {
            kind: RuntimeConfigSettingsErrorKind::Internal,
            failure: Box::new(agena_failure::Failure::new(
                agena_failure::FailureCode::new("settings.internal"),
                agena_failure::FailureCategory::Internal,
                agena_failure::FailureResponsibility::System,
                agena_failure::RetryDirective::Unknown,
                agena_failure::RecoveryDirective::Retry,
                agena_failure::FailureImpact::OperationFailed,
                presentation,
            )),
            diagnostic,
        }
    }

    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn internal_error_with_context(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain_with_context(
            context, error,
        ))
    }

    pub fn kind(&self) -> RuntimeConfigSettingsErrorKind {
        self.kind
    }

    pub fn failure(&self) -> &agena_failure::Failure {
        &self.failure
    }

    pub fn diagnostic(&self) -> &str {
        self.diagnostic.as_str()
    }
}

impl std::fmt::Display for RuntimeConfigSettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.failure.user.fallback.as_str())
    }
}

impl std::error::Error for RuntimeConfigSettingsError {}

/// Service for reading and editing runtime config settings.
pub trait RuntimeConfigSettingsService: Send + Sync {
    fn read_file_settings(
        &self,
        input: ConfigSettingsGetInput,
    ) -> Result<ConfigSettingsReadResponse, RuntimeConfigSettingsError>;
    /// Read the workspace-local configuration file selected by the composed
    /// runtime. The target is deliberately implicit so callers cannot inject
    /// arbitrary concrete configuration paths.
    fn read_project_file_settings(
        &self,
        input: ConfigSettingsGetInput,
    ) -> Result<ConfigSettingsReadResponse, RuntimeConfigSettingsError>;
    fn list_file_settings(
        &self,
        input: ConfigSettingsListInput,
    ) -> Result<ConfigSettingsListResponse, RuntimeConfigSettingsError>;
    fn set_file_setting(
        &self,
        input: ConfigSettingsSetInput,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    /// Write the workspace-local configuration file selected by the composed
    /// runtime without exposing its concrete path to callers.
    fn set_project_file_setting(
        &self,
        input: ConfigSettingsSetInput,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    fn patch_file_settings(
        &self,
        input: ConfigSettingsPatchInput,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    fn delete_file_setting(
        &self,
        input: ConfigSettingsDeleteInput,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    fn delete_project_file_setting(
        &self,
        input: ConfigSettingsDeleteInput,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    fn validate_file_settings(
        &self,
        input: ConfigSettingsValidateInput,
    ) -> Result<ConfigSettingsValidateResponse, RuntimeConfigSettingsError>;
}

/// Read one value from a configuration document without requiring the concrete
/// configuration schema. A missing document is represented as an empty object,
/// matching the settings editor's create-on-first-write behavior.
pub fn read_runtime_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsGetInput,
) -> Result<ConfigSettingsReadResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    let (config_found, document) = read_runtime_settings_document(&config_path)?;
    let value = get_json_path(&document, input.target.path.as_deref())?;
    Ok(ConfigSettingsReadResponse {
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.target.path,
        value,
    })
}

/// List values from a configuration document using Runtime's stable path
/// grammar and transport projection.
pub fn list_runtime_file_settings(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsListInput,
) -> Result<ConfigSettingsListResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    let (config_found, document) = read_runtime_settings_document(&config_path)?;
    let items = list_json_path(&document, input.target.path.as_deref(), input.recursive)?;
    Ok(ConfigSettingsListResponse {
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.target.path,
        items,
    })
}

/// Schema validation is supplied by the concrete composition layer. Runtime
/// owns JSON document operations and persistence mechanics while the composed
/// validator owns schema-specific policy.
pub type RuntimeSettingsDocumentValidator =
    dyn Fn(&Path, &str) -> Result<(), RuntimeConfigSettingsError> + Send + Sync;

pub fn set_runtime_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsSetInput,
    validator: Option<&RuntimeSettingsDocumentValidator>,
) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    crate::with_config_file_write_lock(|| {
        let segments = required_runtime_settings_path_segments(&input.path)?;
        let (config_found, mut document) = read_runtime_settings_document(&config_path)?;
        let before = document.clone();
        let previous = get_json_path(&before, Some(input.path.as_str()))?;
        let created = previous.is_null();
        set_runtime_json_path(&mut document, &segments, input.value)?;
        finish_runtime_settings_edit(
            config_path,
            config_found,
            document,
            before,
            Some(input.path),
            "set",
            input.options,
            created,
            false,
            validator,
        )
    })
}

pub fn delete_runtime_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsDeleteInput,
    validator: Option<&RuntimeSettingsDocumentValidator>,
) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    crate::with_config_file_write_lock(|| {
        let segments = required_runtime_settings_path_segments(&input.path)?;
        let (config_found, mut document) = read_runtime_settings_document(&config_path)?;
        let before = document.clone();
        let deleted = remove_runtime_json_path(&mut document, &segments)?;
        finish_runtime_settings_edit(
            config_path,
            config_found,
            document,
            before,
            Some(input.path),
            "delete",
            input.options,
            false,
            deleted,
            validator,
        )
    })
}

pub fn patch_runtime_file_settings(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsPatchInput,
    validator: Option<&RuntimeSettingsDocumentValidator>,
) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    crate::with_config_file_write_lock(|| {
        let changes = input.changes.as_object().ok_or_else(|| {
            RuntimeConfigSettingsError::invalid_input(
                "settings_patch changes must be a JSON object",
            )
        })?;
        let (config_found, mut document) = read_runtime_settings_document(&config_path)?;
        let before = document.clone();
        let created = input
            .target
            .path
            .as_deref()
            .map(|path| get_json_path(&before, Some(path)).map(|value| value.is_null()))
            .transpose()?
            .unwrap_or(false);
        let target = ensure_runtime_object_path(&mut document, input.target.path.as_deref())?;
        merge_runtime_json_object(target, changes)?;
        finish_runtime_settings_edit(
            config_path,
            config_found,
            document,
            before,
            input.target.path,
            "patch",
            input.options,
            created,
            false,
            validator,
        )
    })
}

pub fn validate_runtime_file_settings(
    config_path: impl Into<PathBuf>,
    validator: &RuntimeSettingsDocumentValidator,
) -> Result<ConfigSettingsValidateResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    let (config_found, document) = read_runtime_settings_document(&config_path)?;
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| RuntimeConfigSettingsError::internal_error(&error))?;
    validator(&config_path, &text)?;
    Ok(ConfigSettingsValidateResponse {
        config_path,
        config_found,
        valid: true,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_runtime_settings_edit(
    config_path: PathBuf,
    config_found: bool,
    document: JsonValue,
    before: JsonValue,
    path: Option<String>,
    operation: &'static str,
    options: ConfigSettingsEditOptions,
    created: bool,
    deleted: bool,
    validator: Option<&RuntimeSettingsDocumentValidator>,
) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError> {
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| RuntimeConfigSettingsError::internal_error(&error))?;
    if options.validate {
        let validator = validator.ok_or_else(|| {
            RuntimeConfigSettingsError::internal(
                "runtime settings edit requested validation without a schema validator",
            )
        })?;
        validator(&config_path, &text)?;
    }
    let previous = get_json_path(&before, path.as_deref())?;
    let current = get_json_path(&document, path.as_deref())?;
    let changed = before != document;
    if changed && !options.dry_run {
        write_runtime_settings_document(&config_path, &text)?;
    }
    Ok(ConfigSettingsEditResponse {
        config_path,
        config_found,
        operation: operation.to_string(),
        path,
        dry_run: options.dry_run,
        changed,
        created,
        deleted,
        validated: options.validate,
        reload_requested: options.reload,
        reload_required: changed && !options.dry_run && options.reload,
        reload: None,
        previous,
        current,
    })
}

fn write_runtime_settings_document(
    config_path: &Path,
    text: &str,
) -> Result<(), RuntimeConfigSettingsError> {
    crate::write_config_file_atomically(config_path, text.as_bytes()).map_err(|error| {
        RuntimeConfigSettingsError::internal(format!(
            "failed to write configuration file {}: {error}",
            config_path.display()
        ))
    })
}

fn required_runtime_settings_path_segments(
    path: &str,
) -> Result<Vec<String>, RuntimeConfigSettingsError> {
    let segments = parse_runtime_settings_path(path)?;
    if segments.is_empty() {
        return Err(RuntimeConfigSettingsError::invalid_input(
            "settings path must not be empty",
        ));
    }
    Ok(segments)
}

fn ensure_runtime_object_path<'a>(
    root: &'a mut JsonValue,
    path: Option<&str>,
) -> Result<&'a mut JsonMap<String, JsonValue>, RuntimeConfigSettingsError> {
    if !root.is_object() {
        *root = JsonValue::Object(JsonMap::new());
    }
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return root.as_object_mut().ok_or_else(|| {
            RuntimeConfigSettingsError::invalid_input("settings root must be an object")
        });
    };
    let segments = parse_runtime_settings_path(path)?;
    let mut cursor = root;
    for segment in segments {
        let object = cursor.as_object_mut().ok_or_else(|| {
            RuntimeConfigSettingsError::invalid_input(format!(
                "settings path `{path}` crosses non-object segment `{segment}`"
            ))
        })?;
        let child = object
            .entry(segment)
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        if child.is_null() {
            *child = JsonValue::Object(JsonMap::new());
        }
        if !child.is_object() {
            return Err(RuntimeConfigSettingsError::invalid_input(format!(
                "settings path `{path}` crosses non-object segment"
            )));
        }
        cursor = child;
    }
    cursor.as_object_mut().ok_or_else(|| {
        RuntimeConfigSettingsError::invalid_input("settings target must be an object")
    })
}

fn set_runtime_json_path(
    root: &mut JsonValue,
    segments: &[String],
    value: JsonValue,
) -> Result<(), RuntimeConfigSettingsError> {
    let key = segments
        .last()
        .ok_or_else(|| {
            RuntimeConfigSettingsError::invalid_input("settings path must not be empty")
        })?
        .clone();
    let parent_path =
        (segments.len() > 1).then(|| format_settings_path(&segments[..segments.len() - 1]));
    let object = ensure_runtime_object_path(root, parent_path.as_deref())?;
    object.insert(key, value);
    Ok(())
}

fn remove_runtime_json_path(
    root: &mut JsonValue,
    segments: &[String],
) -> Result<bool, RuntimeConfigSettingsError> {
    let key = segments.last().ok_or_else(|| {
        RuntimeConfigSettingsError::invalid_input("settings path must not be empty")
    })?;
    if segments.len() == 1 {
        return Ok(root
            .as_object_mut()
            .is_some_and(|object| object.remove(key.as_str()).is_some()));
    }
    let mut cursor = root;
    for segment in &segments[..segments.len() - 1] {
        let Some(object) = cursor.as_object_mut() else {
            return Ok(false);
        };
        let Some(child) = object.get_mut(segment.as_str()) else {
            return Ok(false);
        };
        cursor = child;
    }
    Ok(cursor
        .as_object_mut()
        .is_some_and(|object| object.remove(key.as_str()).is_some()))
}

fn merge_runtime_json_object(
    target: &mut JsonMap<String, JsonValue>,
    changes: &JsonMap<String, JsonValue>,
) -> Result<(), RuntimeConfigSettingsError> {
    for (key, value) in changes {
        if value.is_null() {
            target.remove(key.as_str());
            continue;
        }
        if let Some(object_patch) = value.as_object() {
            let entry = target
                .entry(key.clone())
                .or_insert_with(|| JsonValue::Object(JsonMap::new()));
            if entry.is_null() {
                *entry = JsonValue::Object(JsonMap::new());
            }
            let child = entry.as_object_mut().ok_or_else(|| {
                RuntimeConfigSettingsError::invalid_input(format!(
                    "settings_patch cannot merge object into non-object key `{key}`"
                ))
            })?;
            merge_runtime_json_object(child, object_patch)?;
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

fn read_runtime_settings_document(
    config_path: &PathBuf,
) -> Result<(bool, JsonValue), RuntimeConfigSettingsError> {
    match fs::read_to_string(config_path) {
        Ok(text) => {
            let value = serde_json::from_str::<JsonValue>(&text).map_err(|error| {
                RuntimeConfigSettingsError::invalid_input_with_diagnostic(
                    format!(
                        "The configuration JSON is invalid at line {}, column {}.",
                        error.line(),
                        error.column()
                    ),
                    format_args!(
                        "failed to parse configuration file {}: {error}",
                        config_path.display()
                    ),
                )
            })?;
            let value = match value {
                JsonValue::Null => JsonValue::Object(Default::default()),
                other => other,
            };
            Ok((true, value))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((false, JsonValue::Object(Default::default())))
        }
        Err(error) => Err(RuntimeConfigSettingsError::internal(format!(
            "failed to read configuration file {}: {error}",
            config_path.display()
        ))),
    }
}

pub fn get_json_path(
    value: &JsonValue,
    path: Option<&str>,
) -> Result<JsonValue, RuntimeConfigSettingsError> {
    agena_domain::get_json_path(value, path)
        .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))
}

pub fn list_json_path(
    value: &JsonValue,
    path: Option<&str>,
    recursive: bool,
) -> Result<Vec<ConfigSettingsListItem>, RuntimeConfigSettingsError> {
    let base = path.map(str::trim).filter(|path| !path.is_empty());
    let target = get_json_path(value, base)?;
    let base_segments = base
        .map(parse_runtime_settings_path)
        .transpose()?
        .unwrap_or_default();
    let mut entries = Vec::new();
    collect_list_entries(&mut entries, &base_segments, &target, recursive);
    Ok(entries)
}

fn default_true() -> bool {
    true
}

/// Parse a dotted settings path with quoted segments without exposing the
/// concrete Runtime configuration schema.
pub fn parse_runtime_settings_path(path: &str) -> Result<Vec<String>, RuntimeConfigSettingsError> {
    agena_domain::parse_json_path(path)
        .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))
}

fn collect_list_entries(
    entries: &mut Vec<ConfigSettingsListItem>,
    base: &[String],
    value: &JsonValue,
    recursive: bool,
) {
    match value {
        JsonValue::Object(object) => {
            for (key, child) in object {
                let mut child_path = base.to_vec();
                child_path.push(key.clone());
                entries.push(ConfigSettingsListItem {
                    path: format_settings_path(&child_path),
                    kind: json_kind(child).to_owned(),
                    value: scalar_json_value(child),
                });
                if recursive {
                    collect_list_entries(entries, &child_path, child, recursive);
                }
            }
        }
        JsonValue::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let mut child_path = base.to_vec();
                child_path.push(index.to_string());
                entries.push(ConfigSettingsListItem {
                    path: format_settings_path(&child_path),
                    kind: json_kind(child).to_owned(),
                    value: scalar_json_value(child),
                });
                if recursive {
                    collect_list_entries(entries, &child_path, child, recursive);
                }
            }
        }
        other => entries.push(ConfigSettingsListItem {
            path: format_settings_path(base),
            kind: json_kind(other).to_owned(),
            value: Some(other.clone()),
        }),
    }
}

pub fn format_settings_path(segments: &[String]) -> String {
    agena_domain::format_json_path(segments)
}

fn scalar_json_value(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Object(_) | JsonValue::Array(_) => None,
        other => Some(other.clone()),
    }
}

fn json_kind(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "bool",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value as JsonValue, json};

    use super::{
        ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsPatchInput,
        ConfigSettingsPathInput, ConfigSettingsSetInput, delete_runtime_file_setting,
        get_json_path, list_json_path, patch_runtime_file_settings, read_runtime_file_setting,
        set_runtime_file_setting,
    };

    fn no_validation() -> ConfigSettingsEditOptions {
        ConfigSettingsEditOptions {
            dry_run: false,
            validate: false,
            reload: true,
        }
    }

    #[test]
    fn json_path_supports_quoted_object_keys_and_array_indexes() {
        let document = json!({
            "provider settings": { "models": [{ "id": "first" }, { "id": "second" }] }
        });
        assert_eq!(
            get_json_path(&document, Some("\"provider settings\".models.1.id"))
                .expect("read quoted path"),
            json!("second")
        );
    }

    #[test]
    fn list_json_path_keeps_the_existing_scalar_projection_shape() {
        let document = json!({ "config": { "name": "agena", "enabled": true } });
        let entries = list_json_path(&document, Some("config"), false).expect("list config");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "config.enabled");
        assert_eq!(entries[0].kind, "bool");
        assert_eq!(entries[0].value, Some(json!(true)));
        assert_eq!(entries[1].path, "config.name");
        assert_eq!(entries[1].kind, "string");
        assert_eq!(entries[1].value, Some(json!("agena")));
    }

    #[test]
    fn runtime_document_edits_preserve_file_shape_and_edit_metadata() {
        let directory = tempfile::tempdir().expect("create temporary settings directory");
        let path = directory.path().join("nested/settings.json");
        let set = set_runtime_file_setting(
            &path,
            ConfigSettingsSetInput {
                path: "ui.locale".to_owned(),
                value: json!("zh-CN"),
                options: no_validation(),
            },
            None,
        )
        .expect("set runtime setting");
        assert!(set.changed);
        assert!(set.created);
        assert!(set.reload_required);

        let patch = patch_runtime_file_settings(
            &path,
            ConfigSettingsPatchInput {
                target: ConfigSettingsPathInput {
                    path: Some("ui".to_owned()),
                },
                changes: json!({"theme": "night", "locale": null}),
                options: no_validation(),
            },
            None,
        )
        .expect("patch runtime setting");
        assert!(patch.changed);
        assert_eq!(patch.current, json!({"theme": "night"}));

        let read = read_runtime_file_setting(
            &path,
            super::ConfigSettingsGetInput {
                target: ConfigSettingsPathInput {
                    path: Some("ui.theme".to_owned()),
                },
                ..Default::default()
            },
        )
        .expect("read patched setting");
        assert_eq!(read.value, JsonValue::String("night".to_owned()));

        let deleted = delete_runtime_file_setting(
            &path,
            ConfigSettingsDeleteInput {
                path: "ui.theme".to_owned(),
                options: no_validation(),
            },
            None,
        )
        .expect("delete runtime setting");
        assert!(deleted.deleted);
        assert_eq!(
            serde_json::from_str::<JsonValue>(
                &std::fs::read_to_string(path).expect("read runtime settings document"),
            )
            .expect("parse runtime settings document"),
            json!({"ui": {}})
        );
    }
}
