//! Fail-closed boundary for the 17 vendor-hosted capabilities. This layer never
//! invokes Agena tools, executes commands, or forwards client tool results.
use agena_plugin_host::{PluginError, sdk::Result as SdkResult};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn invalid(message: impl Into<String>) -> PluginError {
    PluginError::invalid_params(message.into())
}

pub(crate) fn validate_options(options: &BTreeMap<String, Value>, scope: &str) -> SdkResult<()> {
    fn walk(value: &Value, depth: usize, scope: &str) -> SdkResult<()> {
        if depth > 24 {
            return Err(invalid(format!(
                "{scope} exceeds the supported nesting limit"
            )));
        }
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    if matches!(
                        key.as_str(),
                        "tools"
                            | "functions"
                            | "functionDeclarations"
                            | "function_declarations"
                            | "mcp_servers"
                            | "mcpServers"
                            | "mcp_toolsets"
                            | "toolsets"
                            | "connectors"
                            | "connector_id"
                            | "tunnel_id"
                            | "function_call"
                            | "function_calling_config"
                            | "functionCallingConfig"
                    ) {
                        return Err(invalid(format!(
                            "{scope}.{key} cannot add client tools, remote MCP or a second execution route to a hosted-only request"
                        )));
                    }
                    if matches!(key.as_str(), "type" | "execution")
                        && value.as_str().is_some_and(|kind| {
                            client_type(kind)
                                || matches!(
                                    kind,
                                    "local"
                                        | "client"
                                        | "mcp"
                                        | "mcp_toolset"
                                        | "mcp_server"
                                        | "namespace"
                                        | "programmatic_tool_calling"
                                        | "tool_search"
                                )
                        })
                    {
                        return Err(invalid(format!(
                            "{scope} requests a non-hosted execution mode; use native Agena tools in a separate authorized call"
                        )));
                    }
                    walk(value, depth + 1, scope)?;
                }
            }
            Value::Array(values) => {
                for value in values {
                    walk(value, depth + 1, scope)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(
        &serde_json::to_value(options).map_err(|error| PluginError::internal_error(&error))?,
        0,
        scope,
    )
}

pub(crate) fn validate_declaration(
    provider: &str,
    tool: &str,
    declaration: &Value,
) -> SdkResult<()> {
    if agena_tool::provider_tools::cloud_operation(provider, tool).is_none() {
        return Err(invalid(
            "this provider operation is not registered as a hosted capability",
        ));
    }
    if provider == "chatgpt" && tool == "shell" {
        let env=declaration.get("environment").and_then(Value::as_object)
            .ok_or_else(||invalid("chatgpt.shell requires a hosted environment object; local execution is not supported"))?;
        match env.get("type").and_then(Value::as_str) {
            Some("container_auto") => {
                if env.contains_key("container_id") {
                    return Err(invalid(
                        "container_auto must not include container_id; use container_reference",
                    ));
                }
            }
            Some("container_reference") => {
                if !env
                    .get("container_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("cntr_") && id.len() > 5)
                {
                    return Err(invalid(
                        "container_reference requires a nonempty OpenAI cntr_ container_id",
                    ));
                }
            }
            _ => {
                return Err(invalid(
                    "chatgpt.shell permits container_auto or container_reference only; never local/custom environments",
                ));
            }
        }
    }
    if provider == "chatgpt" && tool == "code_interpreter" {
        let value = &declaration["container"];
        if !(value
            .as_str()
            .is_some_and(|id| id.starts_with("cntr_") && id.len() > 5)
            || value.get("type").and_then(Value::as_str) == Some("auto"))
        {
            return Err(invalid(
                "code_interpreter requires an OpenAI container ID or {type: auto}; it cannot use a local runtime",
            ));
        }
    }
    if provider == "claude"
        && tool == "advisor"
        && !declaration
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|model| !model.trim().is_empty())
    {
        return Err(invalid(
            "claude.advisor requires tool_options.model identifying the hosted advisor model",
        ));
    }
    Ok(())
}

fn client_type(kind: &str) -> bool {
    matches!(
        kind,
        "function"
            | "custom"
            | "computer"
            | "computer_use"
            | "computer_use_preview"
            | "local_shell"
            | "apply_patch"
            | "bash"
            | "memory"
            | "text_editor"
    ) || kind.starts_with("computer_")
        || kind.starts_with("bash_202")
        || kind.starts_with("memory_202")
        || kind.starts_with("text_editor_202")
}
fn protocol_items<'a>(value: &'a Value, items: &mut Vec<&'a Value>, depth: usize) -> bool {
    if depth > 16 || items.len() >= 4096 {
        return true;
    }
    if let Some(array) = value.as_array() {
        return array
            .iter()
            .any(|item| protocol_items(item, items, depth + 1));
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    items.push(value);
    // Inspect only known protocol envelopes, never arbitrary tool-result
    // data, document JSON, arguments or text. true means inspection was cut
    // short and must fail closed, not certify the unseen remainder.
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind.is_empty()
        || matches!(
            kind,
            "message" | "model_output" | "user_input" | "assistant" | "tool_call"
        )
    {
        for field in [
            "output",
            "outputs",
            "content",
            "steps",
            "parts",
            "candidates",
        ] {
            if object
                .get(field)
                .is_some_and(|child| protocol_items(child, items, depth + 1))
            {
                return true;
            }
        }
    }
    false
}
fn blocked_type(item: &Value) -> Option<&str> {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
    if matches!(
        kind,
        "function_call"
            | "function_result"
            | "function_call_output"
            | "custom_tool_call"
            | "custom_tool_call_output"
            | "local_shell_call"
            | "local_shell_call_output"
            | "apply_patch_call"
            | "apply_patch_call_output"
            | "computer_call"
            | "computer_call_output"
            | "tool_use"
            | "tool_result"
    ) || kind.starts_with("mcp_")
        || kind == "tool_search_call"
        || kind == "tool_search_output"
        || kind == "programmatic_tool_call"
    {
        return Some(kind);
    }
    if item.get("functionCall").is_some() || item.get("functionResponse").is_some() {
        return Some("functionCall/functionResponse");
    }
    None
}
pub(crate) fn validate_history(provider: &str, history: &Value) -> SdkResult<()> {
    let mut items = Vec::new();
    if protocol_items(history, &mut items, 0) {
        return Err(invalid(
            "provider history exceeds the protocol inspection budget; no request was sent",
        ));
    }
    if items.iter().any(|item| blocked_type(item).is_some()) {
        return Err(invalid(format!(
            "{provider} hosted-only history cannot contain application-side tool calls/results or MCP callbacks; do not replay local actions through this plugin"
        )));
    }
    // Hosted Shell continuation uses previous_response_id/container_id, not
    // application-supplied command outputs masquerading as cloud execution.
    if provider == "chatgpt"
        && items.iter().any(|item| {
            matches!(
                item.get("type").and_then(Value::as_str),
                Some("shell_call" | "shell_call_output")
            )
        })
    {
        return Err(invalid(
            "continue hosted shell with previous_response_id and a hosted container; do not submit shell_call/output callbacks",
        ));
    }
    if provider == "claude"
        && items.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("server_tool_use")
                && !server_name(item.get("name").and_then(Value::as_str).unwrap_or_default())
        })
    {
        return Err(invalid(
            "history contains an unsupported server-tool invocation; only hosted search/fetch/code/advisor history is accepted",
        ));
    }
    Ok(())
}
fn server_name(name: &str) -> bool {
    matches!(
        name,
        "web_search"
            | "web_fetch"
            | "code_execution"
            | "bash_code_execution"
            | "text_editor_code_execution"
            | "advisor"
    )
}

#[derive(Debug, Default, Serialize)]
pub(super) struct ExecutionEvidence {
    pub boundary_violations: Vec<String>,
    pub unresolved_hosted_calls: usize,
    pub completed_hosted_calls: usize,
    pub observed_hosted_calls: usize,
    pub server_errors: bool,
}
impl ExecutionEvidence {
    pub fn blocked(&self) -> bool {
        !self.boundary_violations.is_empty()
    }
}
pub(super) fn inspect(provider: &str, tool: &str, response: &Value) -> ExecutionEvidence {
    let mut items = Vec::new();
    let truncated = protocol_items(response, &mut items, 0);
    let mut result = ExecutionEvidence::default();
    let mut violations = BTreeSet::new();
    if truncated {
        violations
            .insert("protocol inspection limit exceeded; response was not fully validated".into());
    }
    let result_ids = items
        .iter()
        .filter_map(|item| {
            let kind = item.get("type")?.as_str()?;
            if kind == "shell_call_output"
                || kind == "code_execution_result"
                || kind.ends_with("_tool_result")
            {
                item.get("call_id")
                    .or_else(|| item.get("tool_use_id"))
                    .and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>();
    for item in items {
        if let Some(kind) = blocked_type(item) {
            violations.insert(kind.to_owned());
            continue;
        }
        let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
        if kind == "server_tool_use" {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            if provider != "claude" || !server_name(name) {
                violations.insert(format!("unsupported server tool: {name}"));
                continue;
            }
            result.observed_hosted_calls += 1;
            if item
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| result_ids.contains(id))
            {
                result.completed_hosted_calls += 1;
            } else {
                result.unresolved_hosted_calls += 1;
            }
        } else if kind == "shell_call" {
            if provider != "chatgpt"
                || tool != "shell"
                || item.pointer("/environment/type").and_then(Value::as_str) == Some("local")
            {
                violations.insert("unexpected shell_call outside hosted shell".into());
                continue;
            }
            result.observed_hosted_calls += 1;
            if item
                .get("call_id")
                .and_then(Value::as_str)
                .is_some_and(|id| result_ids.contains(id))
            {
                result.completed_hosted_calls += 1;
            } else {
                result.unresolved_hosted_calls += 1;
            }
        } else if kind == "code_execution_call" && provider == "gemini" && tool == "code_execution"
        {
            result.observed_hosted_calls += 1;
            if item
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| result_ids.contains(id))
            {
                result.completed_hosted_calls += 1;
            } else {
                result.unresolved_hosted_calls += 1;
            }
        } else if matches!(
            kind,
            "web_search_call"
                | "file_search_call"
                | "code_interpreter_call"
                | "image_generation_call"
        ) {
            result.observed_hosted_calls += 1;
            match item.get("status").and_then(Value::as_str) {
                Some("completed") => result.completed_hosted_calls += 1,
                Some("failed" | "cancelled") => result.server_errors = true,
                _ => result.unresolved_hosted_calls += 1,
            }
        }
        if kind.ends_with("_tool_result") {
            let contents = item.get("content");
            let error = |part: &Value| {
                part.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.ends_with("_error"))
                    || part.get("is_error").and_then(Value::as_bool) == Some(true)
            };
            if item.get("is_error").and_then(Value::as_bool) == Some(true)
                || contents
                    .is_some_and(|v| error(v) || v.as_array().is_some_and(|a| a.iter().any(error)))
            {
                result.server_errors = true;
            }
        }
    }
    result.boundary_violations = violations.into_iter().take(128).collect();
    result
}
