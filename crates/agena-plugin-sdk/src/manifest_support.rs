use serde_json::Value;

use crate::manifest::HookSubscription;
use crate::manifest::ToolTag;

pub fn normalize_tool_tag_name(tag: impl AsRef<str>) -> Option<String> {
    let normalized = tag
        .as_ref()
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    (!normalized.is_empty()).then_some(normalized)
}

pub(crate) fn normalize_schema_json(value: Value) -> Value {
    normalize_schema_json_value(value, true)
}

pub(crate) fn serde_json_value_is_empty_schema(value: &Value) -> bool {
    matches!(value, Value::Null) || value.as_object().is_some_and(|object| object.is_empty())
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
                cleaned.insert("properties".to_string(), Value::Object(serde_json::Map::new()));
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

fn push_normalized_tag(tags: &mut Vec<ToolTag>, tag: ToolTag) {
    if !tags.iter().any(|existing| existing == &tag) {
        tags.push(tag);
    }
}

pub(crate) fn normalize_tags<I>(tags: I) -> Vec<ToolTag>
where
    I: IntoIterator<Item = ToolTag>,
{
    let mut normalized = Vec::new();
    for tag in tags {
        push_normalized_tag(&mut normalized, tag);
    }
    normalized
}

pub(crate) fn hook_subscription_for_name(name: &str) -> Option<HookSubscription> {
    const HOOK_NAMES: &[(&str, HookSubscription)] = &[
        ("init", HookSubscription::INIT),
        ("shutdown", HookSubscription::SHUTDOWN),
        ("tool.execute.before", HookSubscription::TOOL_BEFORE),
        ("tool.execute.after", HookSubscription::TOOL_AFTER),
        ("tool.execute.failure", HookSubscription::TOOL_FAILURE),
        ("tool.invoke", HookSubscription::TOOL_INVOKE),
        ("tool.invoke.stream", HookSubscription::TOOL_INVOKE_STREAM),
        ("tool.definition", HookSubscription::TOOL_DEFINITION),
        ("event", HookSubscription::EVENT),
        ("chat.message", HookSubscription::CHAT_MESSAGE),
        (
            "chat.messages.transform",
            HookSubscription::CHAT_MESSAGES_TRANSFORM,
        ),
        ("chat.params", HookSubscription::CHAT_PARAMS),
        ("chat.headers", HookSubscription::CHAT_HEADERS),
        (
            "chat.system.transform",
            HookSubscription::CHAT_SYSTEM_TRANSFORM,
        ),
        ("auth", HookSubscription::AUTH),
        ("provider.list", HookSubscription::PROVIDER_LIST),
        (
            "permission.ask_permission",
            HookSubscription::PERMISSION_ASK,
        ),
        ("notification", HookSubscription::NOTIFICATION),
        ("command.execute.before", HookSubscription::COMMAND_BEFORE),
        ("command.execute.after", HookSubscription::COMMAND_AFTER),
        ("shell.env", HookSubscription::SHELL_ENV),
        ("config", HookSubscription::CONFIG),
        ("session.start", HookSubscription::SESSION_START),
        ("session.end", HookSubscription::SESSION_END),
        ("user.prompt.submit", HookSubscription::USER_PROMPT_SUBMIT),
        ("agent.stop", HookSubscription::AGENT_STOP),
        ("pre_run", HookSubscription::PRE_RUN),
        ("post_run", HookSubscription::POST_RUN),
    ];

    HOOK_NAMES
        .iter()
        .find_map(|(hook_name, flag)| (*hook_name == name).then_some(*flag))
}
