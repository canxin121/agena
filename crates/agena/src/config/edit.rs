use std::{fs, path::PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

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
pub struct ConfigSettingsGetInput {
    pub path: Option<String>,
    pub source: ConfigSettingsSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ConfigSettingsListInput {
    pub path: Option<String>,
    pub source: ConfigSettingsSource,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSettingsSetInput {
    pub path: String,
    pub value: JsonValue,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_true")]
    pub validate: bool,
    #[serde(default = "default_true")]
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSettingsDeleteInput {
    pub path: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_true")]
    pub validate: bool,
    #[serde(default = "default_true")]
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSettingsPatchInput {
    #[serde(default)]
    pub path: Option<String>,
    pub changes: JsonValue,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_true")]
    pub validate: bool,
    #[serde(default = "default_true")]
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ConfigSettingsValidateInput {
    pub path: Option<String>,
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
pub struct ConfigSettingsListEntry {
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
    pub entries: Vec<ConfigSettingsListEntry>,
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
) -> Result<Vec<ConfigSettingsListEntry>, ConfigError> {
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
    entries: &mut Vec<ConfigSettingsListEntry>,
    base: &[String],
    value: &JsonValue,
    recursive: bool,
) {
    match value {
        JsonValue::Object(object) => {
            for (key, child) in object {
                let mut child_path = base.to_vec();
                child_path.push(key.clone());
                entries.push(ConfigSettingsListEntry {
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
                entries.push(ConfigSettingsListEntry {
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
            entries.push(ConfigSettingsListEntry {
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
    let (config_found, doc) = read_or_create_doc(&config_path)?;
    let file_value = doc_to_json(&doc)?;
    let value = get_json_path(&file_value, input.path.as_deref())?;
    Ok(ConfigSettingsReadResponse {
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.path,
        value,
    })
}

pub fn list_file_settings(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsListInput,
) -> Result<ConfigSettingsListResponse, ConfigError> {
    let config_path = config_path.into();
    let (config_found, doc) = read_or_create_doc(&config_path)?;
    let file_value = doc_to_json(&doc)?;
    let entries = list_json_path(&file_value, input.path.as_deref(), input.recursive)?;
    Ok(ConfigSettingsListResponse {
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.path,
        entries,
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
    if input.value.is_null() {
        return Err(ConfigError::Validation(
            "settings_set cannot write null; use settings_delete or settings_patch null entries"
                .to_owned(),
        ));
    }
    let config_path = config_path.into();
    let segments = required_path_segments(&input.path)?;
    let (config_found, mut doc) = read_or_create_doc(&config_path)?;
    let before = doc_to_json(&doc)?;
    let previous = get_json_path(&before, Some(input.path.as_str()))?;
    let created = previous.is_null();
    let item = json_to_item(&input.value)?;
    set_doc_item(&mut doc, &segments, item)?;
    finish_edit(
        config_path,
        config_found,
        doc,
        before,
        Some(input.path),
        "set",
        input.dry_run,
        input.validate,
        input.reload,
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
    let before = doc_to_json(&doc)?;
    let deleted = remove_doc_item(&mut doc, &segments)?;
    finish_edit(
        config_path,
        config_found,
        doc,
        before,
        Some(input.path),
        "delete",
        input.dry_run,
        input.validate,
        input.reload,
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
    let before = doc_to_json(&doc)?;
    let created = match input.path.as_deref() {
        Some(path) => get_json_path(&before, Some(path))?.is_null(),
        None => false,
    };
    let target = ensure_doc_table(&mut doc, input.path.as_deref())?;
    merge_json_object_into_table(target, changes)?;
    finish_edit(
        config_path,
        config_found,
        doc,
        before,
        input.path,
        "patch",
        input.dry_run,
        input.validate,
        input.reload,
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
    let text = doc.to_string();
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
    doc: DocumentMut,
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
    let text = doc.to_string();
    if validate {
        super::raw::validate_config_text(&config_path, text.as_str(), env)?;
    }
    let after = doc_to_json(&doc)?;
    let previous = get_json_path(&before, path.as_deref())?;
    let current = get_json_path(&after, path.as_deref())?;
    let changed = before != after;
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

fn read_or_create_doc(path: &PathBuf) -> Result<(bool, DocumentMut), ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let doc = text.parse::<DocumentMut>().map_err(|err| {
                ConfigError::Validation(format!(
                    "failed to parse editable config file {}: {err}",
                    path.display()
                ))
            })?;
            Ok((true, doc))
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok((false, DocumentMut::new()))
        }
        Err(source) => Err(ConfigError::ReadFile {
            path: path.clone(),
            source,
        }),
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

fn doc_to_json(doc: &DocumentMut) -> Result<JsonValue, ConfigError> {
    let text = doc.to_string();
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let parsed =
        toml::from_str::<toml::Value>(text.as_str()).map_err(|source| ConfigError::ParseFile {
            path: PathBuf::from("<memory>"),
            source,
        })?;
    serde_json::to_value(parsed).map_err(ConfigError::from)
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

fn ensure_doc_table<'a>(
    doc: &'a mut DocumentMut,
    path: Option<&str>,
) -> Result<&'a mut Table, ConfigError> {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(doc.as_table_mut());
    };
    let segments = parse_settings_path(path)?;
    let mut table = doc.as_table_mut();
    for segment in segments {
        let item = table
            .entry(segment.as_str())
            .or_insert(Item::Table(Table::new()));
        if item.is_none() {
            *item = Item::Table(Table::new());
        }
        table = item.as_table_mut().ok_or_else(|| {
            ConfigError::Validation(format!(
                "settings path `{path}` crosses non-table segment `{segment}`"
            ))
        })?;
    }
    Ok(table)
}

fn ensure_parent_table<'a>(
    doc: &'a mut DocumentMut,
    segments: &[String],
) -> Result<&'a mut Table, ConfigError> {
    let mut table = doc.as_table_mut();
    for segment in &segments[..segments.len().saturating_sub(1)] {
        let item = table
            .entry(segment.as_str())
            .or_insert(Item::Table(Table::new()));
        if item.is_none() {
            *item = Item::Table(Table::new());
        }
        table = item.as_table_mut().ok_or_else(|| {
            ConfigError::Validation(format!(
                "settings path crosses non-table segment `{segment}`"
            ))
        })?;
    }
    Ok(table)
}

fn set_doc_item(doc: &mut DocumentMut, segments: &[String], item: Item) -> Result<(), ConfigError> {
    let parent = ensure_parent_table(doc, segments)?;
    let key = segments
        .last()
        .ok_or_else(|| ConfigError::Validation("settings path must not be empty".to_owned()))?;
    parent.insert(key.as_str(), item);
    Ok(())
}

fn remove_doc_item(doc: &mut DocumentMut, segments: &[String]) -> Result<bool, ConfigError> {
    let parent = ensure_parent_table(doc, segments)?;
    let key = segments
        .last()
        .ok_or_else(|| ConfigError::Validation("settings path must not be empty".to_owned()))?;
    Ok(parent.remove(key.as_str()).is_some())
}

fn merge_json_object_into_table(
    table: &mut Table,
    object: &serde_json::Map<String, JsonValue>,
) -> Result<(), ConfigError> {
    for (key, value) in object {
        if value.is_null() {
            table.remove(key.as_str());
        } else if let Some(child) = value.as_object() {
            let item = table
                .entry(key.as_str())
                .or_insert(Item::Table(Table::new()));
            if item.is_none() {
                *item = Item::Table(Table::new());
            }
            let table = item.as_table_mut().ok_or_else(|| {
                ConfigError::Validation(format!(
                    "settings_patch cannot merge object into non-table key `{key}`"
                ))
            })?;
            merge_json_object_into_table(table, child)?;
        } else {
            table.insert(key.as_str(), Item::Value(json_to_toml_value(value)?));
        }
    }
    Ok(())
}

fn json_to_item(value: &JsonValue) -> Result<Item, ConfigError> {
    match value {
        JsonValue::Object(object) => {
            let mut table = Table::new();
            merge_json_object_into_table(&mut table, object)?;
            Ok(Item::Table(table))
        }
        other => Ok(Item::Value(json_to_toml_value(other)?)),
    }
}

fn json_to_toml_value(value: &JsonValue) -> Result<TomlValue, ConfigError> {
    match value {
        JsonValue::Null => Err(ConfigError::Validation(
            "TOML settings cannot represent JSON null".to_owned(),
        )),
        JsonValue::Bool(value) => Ok(TomlValue::from(*value)),
        JsonValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(TomlValue::from(value))
            } else if let Some(value) = number.as_u64() {
                let value = i64::try_from(value).map_err(|_| {
                    ConfigError::Validation(format!("integer value `{number}` exceeds TOML range"))
                })?;
                Ok(TomlValue::from(value))
            } else {
                let value = number.as_f64().ok_or_else(|| {
                    ConfigError::Validation(format!("invalid numeric value `{number}`"))
                })?;
                if !value.is_finite() {
                    return Err(ConfigError::Validation(format!(
                        "numeric value `{number}` is not finite"
                    )));
                }
                Ok(TomlValue::from(value))
            }
        }
        JsonValue::String(value) => Ok(TomlValue::from(value.clone())),
        JsonValue::Array(items) => {
            let mut array = Array::new();
            for item in items {
                array.push_formatted(json_to_toml_value(item)?);
            }
            Ok(TomlValue::from(array))
        }
        JsonValue::Object(object) => {
            let mut table = InlineTable::new();
            for (key, value) in object {
                if value.is_null() {
                    return Err(ConfigError::Validation(format!(
                        "TOML inline table key `{key}` cannot be null"
                    )));
                }
                table.insert(key.as_str(), json_to_toml_value(value)?);
            }
            Ok(TomlValue::from(table))
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn config_path() -> (TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("tempdir should create");
        let path = temp.path().join("config.toml");
        (temp, path)
    }

    #[test]
    fn parse_settings_path_supports_quoted_segments() {
        assert_eq!(
            parse_settings_path(r#"plugins.list."agena.mcp".options.servers.filesystem"#).unwrap(),
            vec![
                "plugins",
                "list",
                "agena.mcp",
                "options",
                "servers",
                "filesystem"
            ]
        );
    }

    #[test]
    fn set_file_setting_creates_nested_table() {
        let (_temp, path) = config_path();
        let out = set_file_setting(
            path.clone(),
            ConfigSettingsSetInput {
                path: "tracing.filter".to_string(),
                value: JsonValue::String("debug".to_string()),
                dry_run: false,
                validate: true,
                reload: true,
            },
        )
        .expect("setting should write");

        assert!(out.changed);
        assert!(out.created);
        assert_eq!(out.current, JsonValue::String("debug".to_string()));
        let text = fs::read_to_string(path).expect("config should be readable");
        assert!(text.contains("[tracing]"));
        assert!(text.contains(r#"filter = "debug""#));
    }

    #[test]
    fn patch_file_settings_merges_and_deletes() {
        let (_temp, path) = config_path();
        fs::write(
            &path,
            r#"
[runtime.provider_http]
timeout_secs = 120
connect_timeout_secs = 15
"#,
        )
        .unwrap();

        let out = patch_file_settings(
            path.clone(),
            ConfigSettingsPatchInput {
                path: Some("runtime.provider_http".to_string()),
                changes: serde_json::json!({
                    "timeout_secs": 90,
                    "connect_timeout_secs": null
                }),
                dry_run: false,
                validate: true,
                reload: true,
            },
        )
        .expect("patch should write");

        assert!(out.changed);
        let text = fs::read_to_string(path).expect("config should be readable");
        assert!(text.contains("timeout_secs = 90"));
        assert!(!text.contains("connect_timeout_secs"));
    }

    #[test]
    fn delete_file_setting_removes_key() {
        let (_temp, path) = config_path();
        fs::write(
            &path,
            r#"
[ui]
locale = "en-US"
"#,
        )
        .unwrap();

        let out = delete_file_setting(
            path.clone(),
            ConfigSettingsDeleteInput {
                path: "ui.locale".to_string(),
                dry_run: false,
                validate: true,
                reload: true,
            },
        )
        .expect("delete should write");

        assert!(out.deleted);
        assert_eq!(out.previous, JsonValue::String("en-US".to_string()));
        assert_eq!(out.current, JsonValue::Null);
    }

    #[test]
    fn validation_rejects_invalid_runtime_values() {
        let (_temp, path) = config_path();
        let err = set_file_setting(
            path,
            ConfigSettingsSetInput {
                path: "runtime.reload.poll_interval_secs".to_string(),
                value: JsonValue::Number(0.into()),
                dry_run: true,
                validate: true,
                reload: false,
            },
        )
        .expect_err("invalid runtime value should fail validation");

        assert!(err.to_string().contains("poll_interval_secs"));
    }
}
