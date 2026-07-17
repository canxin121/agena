use serde_json::Value;

pub(crate) fn normalize_schema_json(value: Value) -> Value {
    normalize_schema_json_value(value, true)
}

fn normalize_schema_json_value(value: Value, remove_schema_metadata: bool) -> Value {
    match value {
        Value::Object(mut object) => {
            if remove_schema_metadata {
                object.remove("$schema");
                object.remove("title");
            }
            let mut cleaned = serde_json::Map::new();
            for (key, value) in object {
                let normalized = match key.as_str() {
                    "properties" => match value {
                        Value::Object(map) => Value::Object(
                            map.into_iter()
                                .map(|(nested_key, nested_value)| {
                                    (nested_key, normalize_schema_json_value(nested_value, true))
                                })
                                .collect(),
                        ),
                        other => normalize_schema_json_value(other, true),
                    },
                    "required" => match value {
                        Value::Array(items) => Value::Array(items),
                        other => normalize_schema_json_value(other, true),
                    },
                    "$defs" | "definitions" | "patternProperties" | "dependentSchemas" => {
                        match value {
                            Value::Object(map) => Value::Object(
                                map.into_iter()
                                    .map(|(nested_key, nested_value)| {
                                        (
                                            nested_key,
                                            normalize_schema_json_value(nested_value, true),
                                        )
                                    })
                                    .collect(),
                            ),
                            other => normalize_schema_json_value(other, true),
                        }
                    }
                    _ => normalize_schema_json_value(value, true),
                };
                cleaned.insert(key, normalized);
            }
            if !cleaned.contains_key("type") && schema_map_is_object_like(&cleaned) {
                cleaned.insert("type".to_string(), Value::String("object".to_string()));
            }
            if cleaned
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "object")
                && !cleaned.contains_key("properties")
            {
                cleaned.insert(
                    "properties".to_string(),
                    Value::Object(serde_json::Map::new()),
                );
            }
            Value::Object(cleaned)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| normalize_schema_json_value(item, true))
                .collect(),
        ),
        other => other,
    }
}

fn schema_map_is_object_like(map: &serde_json::Map<String, Value>) -> bool {
    if map
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "object")
    {
        return true;
    }
    if map.contains_key("properties") || map.contains_key("required") {
        return true;
    }
    ["oneOf", "anyOf", "allOf"].into_iter().any(|key| {
        map.get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty() && items.iter().all(schema_value_is_object_like))
    })
}

fn schema_value_is_object_like(value: &Value) -> bool {
    value.as_object().is_some_and(schema_map_is_object_like)
}
