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
