use std::{fs, path::PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::{ConfigEnvironment, ConfigError, ProcessEnvironment};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSettingsSource {
    #[default]
    Effective,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ConfigSettingsPathInput {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ConfigSettingsEditOptions {
    pub dry_run: bool,
    #[serde(default = "default_true")]
    pub validate: bool,
    #[serde(default = "default_true")]
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ConfigSettingsGetInput {
    #[serde(flatten)]
    pub target: ConfigSettingsPathInput,
    pub source: ConfigSettingsSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ConfigSettingsListInput {
    #[serde(flatten)]
    pub target: ConfigSettingsPathInput,
    pub source: ConfigSettingsSource,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSettingsSetInput {
    pub path: String,
    pub value: JsonValue,
    #[serde(flatten)]
    pub options: ConfigSettingsEditOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSettingsDeleteInput {
    pub path: String,
    #[serde(flatten)]
    pub options: ConfigSettingsEditOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSettingsPatchInput {
    #[serde(flatten)]
    pub target: ConfigSettingsPathInput,
    pub changes: JsonValue,
    #[serde(flatten)]
    pub options: ConfigSettingsEditOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ConfigSettingsValidateInput {
    #[serde(flatten)]
    pub target: ConfigSettingsPathInput,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSettingsReadResponse {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub source: ConfigSettingsSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub value: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSettingsListItem {
    pub path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<JsonValue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSettingsListResponse {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub source: ConfigSettingsSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub items: Vec<ConfigSettingsListItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSettingsEditResponse {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub operation: &'static str,
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

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSettingsReloadResponse {
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSettingsValidateResponse {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub valid: bool,
}

fn default_true() -> bool {
    true
}

pub fn parse_settings_path(path: &str) -> Result<Vec<String>, ConfigError> {
    let input = path.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut quoted_segment = false;

    for ch in input.chars() {
        if let Some(quote_char) = quote {
            if escaped {
                current.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote_char {
                quote = None;
                quoted_segment = true;
                continue;
            }
            current.push(ch);
            continue;
        }

        match ch {
            '.' => {
                push_path_segment(&mut segments, &current, quoted_segment)?;
                current.clear();
                quoted_segment = false;
            }
            '"' | '\'' if current.trim().is_empty() => {
                current.clear();
                quote = Some(ch);
            }
            other => current.push(other),
        }
    }

    if escaped {
        return Err(ConfigError::Validation(format!(
            "invalid settings path `{path}`: trailing escape"
        )));
    }
    if quote.is_some() {
        return Err(ConfigError::Validation(format!(
            "invalid settings path `{path}`: unterminated quoted segment"
        )));
    }

    push_path_segment(&mut segments, &current, quoted_segment)?;
    Ok(segments)
}

fn push_path_segment(
    segments: &mut Vec<String>,
    segment: &str,
    quoted: bool,
) -> Result<(), ConfigError> {
    let segment = if quoted {
        segment.to_string()
    } else {
        segment.trim().to_string()
    };
    if segment.is_empty() {
        return Err(ConfigError::Validation(
            "settings path segments must not be empty".to_owned(),
        ));
    }
    segments.push(segment);
    Ok(())
}

pub fn get_json_path(value: &JsonValue, path: Option<&str>) -> Result<JsonValue, ConfigError> {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(value.clone());
    };
    let segments = parse_settings_path(path)?;
    let mut cursor = value;
    for segment in segments {
        cursor = match cursor {
            JsonValue::Object(object) => match object.get(segment.as_str()) {
                Some(value) => value,
                None => return Ok(JsonValue::Null),
            },
            JsonValue::Array(items) => {
                let Ok(index) = segment.parse::<usize>() else {
                    return Ok(JsonValue::Null);
                };
                match items.get(index) {
                    Some(value) => value,
                    None => return Ok(JsonValue::Null),
                }
            }
            _ => return Ok(JsonValue::Null),
        };
    }
    Ok(cursor.clone())
}

pub fn list_json_path(
    value: &JsonValue,
    path: Option<&str>,
    recursive: bool,
) -> Result<Vec<ConfigSettingsListItem>, ConfigError> {
    let base = path.map(str::trim).filter(|path| !path.is_empty());
    let target = get_json_path(value, base)?;
    let base_segments = match base {
        Some(path) => parse_settings_path(path)?,
        None => Vec::new(),
    };
    let mut entries = Vec::new();
    collect_list_entries(&mut entries, &base_segments, &target, recursive);
    Ok(entries)
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
                    kind: json_kind(child).to_string(),
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
                    kind: json_kind(child).to_string(),
                    value: scalar_json_value(child),
                });
                if recursive {
                    collect_list_entries(entries, &child_path, child, recursive);
                }
            }
        }
        other => {
            entries.push(ConfigSettingsListItem {
                path: format_settings_path(base),
                kind: json_kind(other).to_string(),
                value: Some(other.clone()),
            });
        }
    }
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

pub fn format_settings_path(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| {
            if segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            {
                segment.clone()
            } else {
                format!("\"{}\"", segment.replace('\\', "\\\\").replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

pub fn read_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsGetInput,
) -> Result<ConfigSettingsReadResponse, ConfigError> {
    let config_path = config_path.into();
    let (config_found, file_value) = read_or_create_doc(&config_path)?;
    let value = get_json_path(&file_value, input.target.path.as_deref())?;
    Ok(ConfigSettingsReadResponse {
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.target.path,
        value,
    })
}

pub fn list_file_settings(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsListInput,
) -> Result<ConfigSettingsListResponse, ConfigError> {
    let config_path = config_path.into();
    let (config_found, file_value) = read_or_create_doc(&config_path)?;
    let items = list_json_path(&file_value, input.target.path.as_deref(), input.recursive)?;
    Ok(ConfigSettingsListResponse {
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.target.path,
        items,
    })
}

pub fn set_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsSetInput,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    set_file_setting_with_env(config_path, input, &ProcessEnvironment)
}

pub fn set_file_setting_with_env(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsSetInput,
    env: &dyn ConfigEnvironment,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    let config_path = config_path.into();
    let segments = required_path_segments(&input.path)?;
    let (config_found, mut doc) = read_or_create_doc(&config_path)?;
    let before = doc.clone();
    let previous = get_json_path(&before, Some(input.path.as_str()))?;
    let created = previous.is_null();
    set_json_path(&mut doc, &segments, input.value)?;
    finish_edit(
        config_path,
        config_found,
        doc,
        before,
        Some(input.path),
        "set",
        input.options.dry_run,
        input.options.validate,
        input.options.reload,
        created,
        false,
        env,
    )
}

pub fn delete_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsDeleteInput,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    delete_file_setting_with_env(config_path, input, &ProcessEnvironment)
}

pub fn delete_file_setting_with_env(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsDeleteInput,
    env: &dyn ConfigEnvironment,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    let config_path = config_path.into();
    let segments = required_path_segments(&input.path)?;
    let (config_found, mut doc) = read_or_create_doc(&config_path)?;
    let before = doc.clone();
    let deleted = remove_json_path(&mut doc, &segments)?;
    finish_edit(
        config_path,
        config_found,
        doc,
        before,
        Some(input.path),
        "delete",
        input.options.dry_run,
        input.options.validate,
        input.options.reload,
        false,
        deleted,
        env,
    )
}

pub fn patch_file_settings(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsPatchInput,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    patch_file_settings_with_env(config_path, input, &ProcessEnvironment)
}

pub fn patch_file_settings_with_env(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsPatchInput,
    env: &dyn ConfigEnvironment,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    let changes = input.changes.as_object().ok_or_else(|| {
        ConfigError::Validation("settings_patch changes must be a JSON object".to_owned())
    })?;
    let config_path = config_path.into();
    let (config_found, mut doc) = read_or_create_doc(&config_path)?;
    let before = doc.clone();
    let created = match input.target.path.as_deref() {
        Some(path) => get_json_path(&before, Some(path))?.is_null(),
        None => false,
    };
    let target = ensure_object_path(&mut doc, input.target.path.as_deref())?;
    merge_json_object(target, changes)?;
    finish_edit(
        config_path,
        config_found,
        doc,
        before,
        input.target.path,
        "patch",
        input.options.dry_run,
        input.options.validate,
        input.options.reload,
        created,
        false,
        env,
    )
}

pub fn validate_file_settings(
    config_path: impl Into<PathBuf>,
) -> Result<ConfigSettingsValidateResponse, ConfigError> {
    validate_file_settings_with_env(config_path, &ProcessEnvironment)
}

pub fn validate_file_settings_with_env(
    config_path: impl Into<PathBuf>,
    env: &dyn ConfigEnvironment,
) -> Result<ConfigSettingsValidateResponse, ConfigError> {
    let config_path = config_path.into();
    let (config_found, doc) = read_or_create_doc(&config_path)?;
    let text = serde_json::to_string_pretty(&doc)?;
    super::raw::validate_config_text(&config_path, text.as_str(), env)?;
    Ok(ConfigSettingsValidateResponse {
        config_path,
        config_found,
        valid: true,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_edit(
    config_path: PathBuf,
    config_found: bool,
    doc: JsonValue,
    before: JsonValue,
    path: Option<String>,
    operation: &'static str,
    dry_run: bool,
    validate: bool,
    reload_requested: bool,
    created: bool,
    deleted: bool,
    env: &dyn ConfigEnvironment,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    let text = serde_json::to_string_pretty(&doc)?;
    if validate {
        super::raw::validate_config_text(&config_path, text.as_str(), env)?;
    }
    let previous = get_json_path(&before, path.as_deref())?;
    let current = get_json_path(&doc, path.as_deref())?;
    let changed = before != doc;
    if changed && !dry_run {
        write_doc(&config_path, text.as_str())?;
    }

    Ok(ConfigSettingsEditResponse {
        config_path,
        config_found,
        operation,
        path,
        dry_run,
        changed,
        created,
        deleted,
        validated: validate,
        reload_requested,
        reload_required: changed && !dry_run && reload_requested,
        reload: None,
        previous,
        current,
    })
}

fn read_or_create_doc(path: &PathBuf) -> Result<(bool, JsonValue), ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let value = serde_json::from_str::<JsonValue>(text.as_str()).map_err(|source| {
                ConfigError::ParseFile {
                    path: path.clone(),
                    source,
                }
            })?;
            Ok((true, normalize_root_object(value)))
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok((false, JsonValue::Object(JsonMap::new())))
        }
        Err(source) => Err(ConfigError::ReadFile {
            path: path.clone(),
            source,
        }),
    }
}

fn normalize_root_object(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(_) => value,
        JsonValue::Null => JsonValue::Object(JsonMap::new()),
        other => other,
    }
}

fn write_doc(path: &PathBuf, text: &str) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::WriteFile {
            path: path.clone(),
            source,
        })?;
    }
    fs::write(path, text).map_err(|source| ConfigError::WriteFile {
        path: path.clone(),
        source,
    })
}

fn required_path_segments(path: &str) -> Result<Vec<String>, ConfigError> {
    let segments = parse_settings_path(path)?;
    if segments.is_empty() {
        return Err(ConfigError::Validation(
            "settings path must not be empty".to_owned(),
        ));
    }
    Ok(segments)
}

fn ensure_object_path<'a>(
    root: &'a mut JsonValue,
    path: Option<&str>,
) -> Result<&'a mut JsonMap<String, JsonValue>, ConfigError> {
    if !root.is_object() {
        *root = JsonValue::Object(JsonMap::new());
    }
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return root
            .as_object_mut()
            .ok_or_else(|| ConfigError::Validation("settings root must be an object".to_owned()));
    };
    let segments = parse_settings_path(path)?;
    let mut cursor = root;
    for segment in segments {
        let object = cursor.as_object_mut().ok_or_else(|| {
            ConfigError::Validation(format!(
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
            return Err(ConfigError::Validation(format!(
                "settings path `{path}` crosses non-object segment"
            )));
        }
        cursor = child;
    }
    cursor
        .as_object_mut()
        .ok_or_else(|| ConfigError::Validation("settings target must be an object".to_owned()))
}

fn set_json_path(
    root: &mut JsonValue,
    segments: &[String],
    value: JsonValue,
) -> Result<(), ConfigError> {
    let key = segments
        .last()
        .ok_or_else(|| ConfigError::Validation("settings path must not be empty".to_owned()))?
        .clone();
    let parent_path = if segments.len() > 1 {
        Some(format_settings_path(&segments[..segments.len() - 1]))
    } else {
        None
    };
    let object = ensure_object_path(root, parent_path.as_deref())?;
    object.insert(key, value);
    Ok(())
}

fn remove_json_path(root: &mut JsonValue, segments: &[String]) -> Result<bool, ConfigError> {
    let key = segments
        .last()
        .ok_or_else(|| ConfigError::Validation("settings path must not be empty".to_owned()))?;
    if segments.len() == 1 {
        let Some(object) = root.as_object_mut() else {
            return Ok(false);
        };
        return Ok(object.remove(key.as_str()).is_some());
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
    let Some(object) = cursor.as_object_mut() else {
        return Ok(false);
    };
    Ok(object.remove(key.as_str()).is_some())
}

fn merge_json_object(
    target: &mut JsonMap<String, JsonValue>,
    changes: &JsonMap<String, JsonValue>,
) -> Result<(), ConfigError> {
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
                ConfigError::Validation(format!(
                    "settings_patch cannot merge object into non-object key `{key}`"
                ))
            })?;
            merge_json_object(child, object_patch)?;
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}
