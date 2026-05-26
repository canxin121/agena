use schemars::{JsonSchema, schema_for};

pub fn json_schema_for<T>() -> serde_json::Value
where
    T: JsonSchema,
{
    let mut value = serde_json::to_value(schema_for!(T))
        .expect("schemars should always serialize generated schema");
    if let Some(object) = value.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }
    sanitize_schema_json(value)
}

pub fn json_schema_for_with_default<T>(default: T) -> serde_json::Value
where
    T: JsonSchema + serde::Serialize,
{
    let mut value = json_schema_for::<T>();
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "default".to_string(),
            serde_json::to_value(default).expect("schema default should serialize"),
        );
    }
    value
}

pub fn empty_config_schema() -> serde_json::Value {
    serde_json::json!({
        "title": "Plugin Config",
        "description": "This plugin does not expose plugin-specific runtime configuration.",
        "type": "object",
        "properties": {},
        "additionalProperties": false,
        "default": {}
    })
}

pub fn set_schema_metadata(
    schema: &mut serde_json::Value,
    pointer: &str,
    title: Option<&str>,
    description: Option<&str>,
) {
    if title.is_none() && description.is_none() {
        return;
    }
    let resolved_pointer = resolve_schema_pointer(schema, pointer)
        .unwrap_or_else(|| panic!("schema pointer `{pointer}` not found"));
    let target = if resolved_pointer.is_empty() {
        schema
    } else {
        schema
            .pointer_mut(resolved_pointer.as_str())
            .unwrap_or_else(|| panic!("schema pointer `{pointer}` not found"))
    };
    let object = target
        .as_object_mut()
        .unwrap_or_else(|| panic!("schema pointer `{pointer}` does not reference an object"));
    if let Some(title) = title {
        object.insert(
            "title".to_string(),
            serde_json::Value::String(title.to_owned()),
        );
    }
    if let Some(description) = description {
        object.insert(
            "description".to_string(),
            serde_json::Value::String(description.to_owned()),
        );
    }
}

fn resolve_schema_pointer(schema: &serde_json::Value, pointer: &str) -> Option<String> {
    if pointer.is_empty() {
        return Some(String::new());
    }
    let mut current = schema;
    let mut resolved_pointer = String::new();
    for segment in pointer
        .split('/')
        .skip(1)
        .map(unescape_json_pointer_segment)
    {
        while let Some((target, target_pointer)) = resolve_schema_ref(schema, current) {
            current = target;
            resolved_pointer = target_pointer.to_owned();
        }
        current = match current {
            serde_json::Value::Object(object) => object.get(segment.as_str())?,
            serde_json::Value::Array(items) => {
                let index = segment.parse::<usize>().ok()?;
                items.get(index)?
            }
            _ => return None,
        };
        resolved_pointer.push('/');
        resolved_pointer.push_str(escape_json_pointer_segment(segment.as_str()).as_str());
    }
    while let Some((target, target_pointer)) = resolve_schema_ref(schema, current) {
        current = target;
        resolved_pointer = target_pointer.to_owned();
    }
    Some(resolved_pointer)
}

fn resolve_schema_ref<'a>(
    root: &'a serde_json::Value,
    current: &'a serde_json::Value,
) -> Option<(&'a serde_json::Value, &'a str)> {
    let target_pointer = current
        .get("$ref")
        .and_then(serde_json::Value::as_str)?
        .strip_prefix('#')?;
    let target = root.pointer(target_pointer)?;
    Some((target, target_pointer))
}

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn unescape_json_pointer_segment(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

fn sanitize_schema_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut object) => {
            object.remove("$schema");
            object.remove("title");
            let mut cleaned = object
                .into_iter()
                .map(|(key, value)| (key, sanitize_schema_json(value)))
                .collect::<serde_json::Map<String, serde_json::Value>>();
            if !cleaned.contains_key("type") && schema_map_is_object_like(&cleaned) {
                cleaned.insert(
                    "type".to_string(),
                    serde_json::Value::String("object".to_string()),
                );
            }
            if cleaned
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "object")
                && !cleaned.contains_key("properties")
            {
                cleaned.insert(
                    "properties".to_string(),
                    serde_json::Value::Object(serde_json::Map::new()),
                );
            }
            serde_json::Value::Object(cleaned)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sanitize_schema_json).collect())
        }
        other => other,
    }
}

fn schema_map_is_object_like(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    if map
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == "object")
    {
        return true;
    }
    if map.contains_key("properties") || map.contains_key("required") {
        return true;
    }
    ["oneOf", "anyOf", "allOf"].into_iter().any(|key| {
        map.get(key)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|items| !items.is_empty() && items.iter().all(schema_value_is_object_like))
    })
}

fn schema_value_is_object_like(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(schema_map_is_object_like)
}

#[cfg(test)]
mod tests {
    use super::{empty_config_schema, set_schema_metadata};

    #[test]
    fn empty_config_schema_describes_absent_plugin_config() {
        let schema = empty_config_schema();
        assert_eq!(
            schema
                .get("description")
                .and_then(serde_json::Value::as_str),
            Some("This plugin does not expose plugin-specific runtime configuration.")
        );
    }

    #[test]
    fn set_schema_metadata_updates_nested_object_nodes() {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "nested": {
                    "type": "object",
                    "properties": {}
                }
            }
        });
        set_schema_metadata(
            &mut schema,
            "/properties/nested",
            Some("Nested"),
            Some("Nested description"),
        );
        let nested = schema.pointer("/properties/nested").unwrap();
        assert_eq!(
            nested.get("title").and_then(serde_json::Value::as_str),
            Some("Nested")
        );
        assert_eq!(
            nested
                .get("description")
                .and_then(serde_json::Value::as_str),
            Some("Nested description")
        );
    }

    #[test]
    fn set_schema_metadata_follows_ref_targets() {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "runtime": {
                    "$ref": "#/$defs/RuntimeConfig"
                }
            },
            "$defs": {
                "RuntimeConfig": {
                    "type": "object",
                    "properties": {
                        "enabled": {
                            "type": "boolean"
                        }
                    }
                }
            }
        });
        set_schema_metadata(
            &mut schema,
            "/properties/runtime/properties/enabled",
            Some("Enabled"),
            Some("Runtime toggle"),
        );
        let enabled = schema
            .pointer("/$defs/RuntimeConfig/properties/enabled")
            .unwrap();
        assert_eq!(
            enabled.get("title").and_then(serde_json::Value::as_str),
            Some("Enabled")
        );
        assert_eq!(
            enabled
                .get("description")
                .and_then(serde_json::Value::as_str),
            Some("Runtime toggle")
        );
    }
}
