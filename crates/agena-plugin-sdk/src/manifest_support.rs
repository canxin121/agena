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
    crate::schema_normalization::normalize_schema_json(value)
}

pub(crate) fn serde_json_value_is_empty_schema(value: &Value) -> bool {
    matches!(value, Value::Null) || value.as_object().is_some_and(|object| object.is_empty())
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
