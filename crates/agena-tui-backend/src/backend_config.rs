use agena_domain::{get_json_path, parse_json_path};
use anyhow::anyhow;
use serde_json::json;

pub(super) fn plugin_config_setting_target(path: &str) -> Result<Option<(String, Vec<String>)>> {
    let segments = parse_json_path(path).map_err(|error| anyhow!(error.to_string()))?;
    if segments.len() < 4
        || segments.first().is_none_or(|segment| segment != "plugins")
        || segments.get(1).is_none_or(|segment| segment != "list")
        || segments.get(3).is_none_or(|segment| segment != "config")
    {
        return Ok(None);
    }
    Ok(Some((segments[2].clone(), segments[4..].to_vec())))
}

pub(super) fn default_static_plugin_record() -> JsonValue {
    json!({
        "enabled": true,
        "package": { "kind": "static" },
        "config": null
    })
}

pub(super) fn plugin_record_for_config_edit(
    sources: &ConfigJsonSources,
    plugin_id: &str,
) -> JsonValue {
    let path = format!("plugins.list.{}", quoted_settings_segment(plugin_id));
    get_json_path(&sources.file, Some(path.as_str()))
        .ok()
        .filter(|value| value.is_object())
        .or_else(|| {
            get_json_path(&sources.effective, Some(path.as_str()))
                .ok()
                .filter(|value| value.is_object())
        })
        .unwrap_or_else(default_static_plugin_record)
}

pub(super) fn normalize_plugin_record_for_config_edit(
    record: &mut JsonValue,
) -> Result<&mut JsonValue> {
    if !record.is_object() {
        *record = default_static_plugin_record();
    }
    let object = record
        .as_object_mut()
        .ok_or_else(|| anyhow!("plugin config record must be an object"))?;
    object
        .entry("enabled".to_owned())
        .or_insert(JsonValue::Bool(true));
    object
        .entry("package".to_owned())
        .or_insert_with(|| json!({ "kind": "static" }));
    Ok(object
        .entry("config".to_owned())
        .or_insert_with(|| JsonValue::Object(JsonMap::new())))
}

pub(super) fn set_nested_json_value(root: &mut JsonValue, segments: &[String], value: JsonValue) {
    if segments.is_empty() {
        *root = value;
        return;
    }
    if !root.is_object() {
        *root = JsonValue::Object(JsonMap::new());
    }
    let mut cursor = root;
    for segment in &segments[..segments.len().saturating_sub(1)] {
        let object = cursor.as_object_mut().expect("nested settings object");
        cursor = object
            .entry(segment.clone())
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        if !cursor.is_object() {
            *cursor = JsonValue::Object(JsonMap::new());
        }
    }
    let object = cursor.as_object_mut().expect("nested settings object");
    object.insert(segments[segments.len() - 1].clone(), value);
}

pub(super) fn remove_nested_json_value(root: &mut JsonValue, segments: &[String]) -> bool {
    if segments.is_empty() {
        let deleted = !root.is_null();
        *root = JsonValue::Null;
        return deleted;
    }
    let mut cursor = root;
    for segment in &segments[..segments.len().saturating_sub(1)] {
        let Some(next) = cursor
            .as_object_mut()
            .and_then(|object| object.get_mut(segment.as_str()))
        else {
            return false;
        };
        cursor = next;
    }
    cursor
        .as_object_mut()
        .and_then(|object| object.remove(segments[segments.len() - 1].as_str()))
        .is_some()
}
use crate::Result;
use crate::{ConfigJsonSources, JsonMap, JsonValue, quoted_settings_segment};
