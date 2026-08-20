pub(crate) fn normalize_path_for_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Resolve every existing component before a path is both authorized and used.
/// For a new output path, canonicalize the nearest existing parent and append
/// the missing suffix. This closes ordinary workspace-symlink escapes while
/// retaining support for creating new files.
pub(super) fn canonicalize_path_for_execution(path: &Path) -> PathBuf {
    crate::canonicalize_mutation_path(path)
}

pub(super) fn truncate_to_char_count(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let Some((idx, _)) = value.char_indices().nth(max_chars) else {
        return value.to_string();
    };
    value[..idx].to_string()
}

pub(super) fn model_output_exceeds_boundary(
    value: &str,
    max_lines: usize,
    max_bytes: usize,
) -> bool {
    line_count(value) > max_lines || value.len() > max_bytes
}

pub(super) fn line_count(value: &str) -> usize {
    value.bytes().filter(|byte| *byte == b'\n').count() + usize::from(!value.is_empty())
}

pub(super) fn bounded_model_output_preview(
    value: &str,
    marker: &str,
    max_lines: usize,
    max_bytes: usize,
) -> String {
    let separator = "\n\n";
    let marker_overhead = marker
        .len()
        .saturating_add(separator.len().saturating_mul(2));
    let content_bytes = max_bytes.saturating_sub(marker_overhead);
    let content_lines = max_lines.saturating_sub(3);
    if content_bytes == 0 || content_lines == 0 {
        return truncate_to_utf8_bytes(marker, max_bytes);
    }

    let lines = value.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return marker.to_string();
    }
    let head_line_count = content_lines.div_ceil(2).min(lines.len());
    let tail_line_count = content_lines
        .saturating_sub(head_line_count)
        .min(lines.len().saturating_sub(head_line_count));
    let head = lines[..head_line_count].join("\n");
    let tail = if tail_line_count == 0 {
        String::new()
    } else {
        lines[lines.len().saturating_sub(tail_line_count)..].join("\n")
    };

    let head_budget = if tail.is_empty() {
        content_bytes
    } else {
        content_bytes.div_ceil(2)
    };
    let tail_budget = content_bytes.saturating_sub(head_budget);
    let head = truncate_to_utf8_bytes(head.as_str(), head_budget);
    let tail = truncate_tail_to_utf8_bytes(tail.as_str(), tail_budget);
    if head.trim().is_empty() && tail.trim().is_empty() {
        return marker.to_string();
    }
    if tail.trim().is_empty() {
        return format!("{head}{separator}{marker}");
    }
    format!("{head}{separator}{marker}{separator}{tail}")
}

pub(super) fn truncate_to_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    let mut end = max_bytes.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

pub(super) fn truncate_tail_to_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    let mut start = value.len().saturating_sub(max_bytes);
    while start < value.len() && !value.is_char_boundary(start) {
        start += 1;
    }
    value[start..].to_string()
}

pub(super) fn access_kind_name(access: AccessKind) -> &'static str {
    match access {
        AccessKind::Read => "read",
        AccessKind::Write => "write",
    }
}

pub(super) fn validate_shell_filesystem_effects(
    tool_name: &str,
    command: &str,
    effects: &FilesystemEffects,
) -> Result<(), ToolError> {
    shell_tools::validate_declared_filesystem_effects(tool_name, command, effects)
}

pub(super) fn shell_command_from_invocation(invocation: &ToolInvocation) -> Option<String> {
    if let Some(payload) = ToolPayloadInput::from_invocation(invocation) {
        let command = match payload {
            ToolPayloadInput::Shell(crate::part::ShellToolInput::Run { command, .. }) => {
                Some(command.command)
            }
            _ => None,
        };
        if command.is_some() {
            return command;
        }
    }
    let value = invocation_input_value(invocation);
    value
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
}

pub(super) fn filesystem_effects_from_input(
    input: &serde_json::Value,
) -> Result<Option<FilesystemEffects>, ToolError> {
    let reads = input.get("reads").or_else(|| input.pointer("/args/reads"));
    let writes = input
        .get("writes")
        .or_else(|| input.pointer("/args/writes"));
    if reads.is_some() || writes.is_some() {
        let read = match reads {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|err| ToolError::invalid_input(format!("reads: {err}")))?,
            None => Vec::new(),
        };
        let write = match writes {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|err| ToolError::invalid_input(format!("writes: {err}")))?,
            None => Vec::new(),
        };
        return Ok(Some(FilesystemEffects { read, write }));
    }
    let Some(value) = input
        .get("filesystem_effects")
        .or_else(|| input.pointer("/args/filesystem_effects"))
    else {
        return Ok(None);
    };
    let effects = serde_json::from_value(value.clone())
        .map_err(|err| ToolError::invalid_input(format!("filesystem_effects: {err}")))?;
    Ok(Some(effects))
}

pub(super) fn invocation_name(invocation: &ToolInvocation) -> String {
    plugin_invocation_name(&PluginInvocation::from_tool_invocation(invocation))
}

pub(super) fn plugin_invocation_name(invocation: &PluginInvocation) -> String {
    invocation.tool_name.clone()
}

pub(super) fn canonical_tool_name(name: &str) -> &str {
    name
}

pub(super) fn resolved_tool_input_value(
    _registered_tool: &RegisteredTool,
    invocation: &ToolInvocation,
) -> serde_json::Value {
    invocation_input_value(invocation)
}

pub(super) fn resolved_plugin_invocation_input_value(
    _registered_tool: &RegisteredTool,
    invocation: &PluginInvocation,
) -> serde_json::Value {
    plugin_invocation_input_value(invocation)
}

pub(super) fn resolve_managed_project_path_alias(
    raw_path: &str,
    workspace_root: &Path,
) -> Option<PathBuf> {
    let normalized = raw_path.trim().replace('\\', "/");
    let prefix = "~/agena/projects/<workspace>";
    let rest = normalized.strip_prefix(prefix)?;
    let rest = rest.trim_start_matches('/');
    let mut resolved = crate::project_state_dir(workspace_root);
    if !rest.is_empty() {
        resolved = resolved.join(rest);
    }
    Some(resolved)
}

pub(super) fn invocation_effective_tags(
    definition: &RegisteredTool,
    _invocation: &ToolInvocation,
) -> Vec<agena_plugin_host::sdk::ToolTag> {
    definition.effective_tags()
}

pub(super) fn is_concurrency_safe_tool_invocation(
    registered_tool: &RegisteredTool,
    _invocation: &PluginInvocation,
) -> bool {
    registered_tool.definition.runtime.concurrency_safe
}

pub(super) fn apply_patch_execution_from_tool_output(
    output: &ToolOutput,
) -> Option<ApplyPatchExecution> {
    let payload = output.to_json_payload()?;
    ApplyPatchExecution::from_tool_payload(&payload)
}

pub(super) fn invocation_input_json(invocation: &ToolInvocation) -> Result<String, ToolError> {
    plugin_invocation_input_json(&PluginInvocation::from_tool_invocation(invocation))
}

pub(super) fn plugin_invocation_input_json(
    invocation: &PluginInvocation,
) -> Result<String, ToolError> {
    serde_json::to_string(&serde_json::Value::from(invocation.input.clone()))
        .map_err(|err| ToolError::invalid_input(err.to_string()))
}

pub(super) fn invocation_input_value(invocation: &ToolInvocation) -> serde_json::Value {
    plugin_invocation_input_value(&PluginInvocation::from_tool_invocation(invocation))
}

pub(super) fn plugin_invocation_input_value(invocation: &PluginInvocation) -> serde_json::Value {
    serde_json::Value::from(invocation.input.clone())
}

pub(super) fn parse_invocation_from_json(
    tool_name: &str,
    input_json: &str,
) -> Result<ToolInvocation, ToolError> {
    let value = if input_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(input_json).map_err(|err| ToolError::invalid_input(err.to_string()))?
    };
    let input = StructuredObject::try_from(value)
        .map_err(|err| ToolError::invalid_input(err.to_string()))?;

    Ok(ToolInvocation {
        tool_api_call: None,
        name: tool_name.to_string(),
        plugin_name: None,
        input,
    })
}

pub(super) fn sdk_path_kind_to_access_kind(kind: SdkPathKind) -> AccessKind {
    match kind {
        SdkPathKind::Read => AccessKind::Read,
        SdkPathKind::Write => AccessKind::Write,
    }
}

pub(super) fn extract_input_path_requests(
    input: &serde_json::Value,
    specs: &[SdkInputPathSpec],
) -> Result<Vec<agena_plugin_host::sdk::PathRequest>, ToolError> {
    let mut requests = Vec::new();
    for spec in specs {
        let matches = extract_jsonpath_values(input, spec.jsonpath.as_str())?;
        if matches.is_empty() {
            if let Some(path) = spec.fallback.as_ref() {
                requests.push(agena_plugin_host::sdk::PathRequest {
                    path: path.clone(),
                    kind: spec.kind,
                });
                continue;
            }
            if spec.optional {
                continue;
            }
            return Err(ToolError::invalid_input(format!(
                "missing required input path '{}'",
                spec.jsonpath
            )));
        }
        for value in matches {
            let Some(path) = value.as_str() else {
                return Err(ToolError::invalid_input(format!(
                    "input path '{}' must resolve to a string",
                    spec.jsonpath
                )));
            };
            requests.push(agena_plugin_host::sdk::PathRequest {
                path: path.to_string(),
                kind: spec.kind,
            });
        }
    }
    Ok(requests)
}

pub(super) fn extract_input_network_requests(
    input: &serde_json::Value,
    specs: &[SdkInputNetworkSpec],
) -> Result<Vec<agena_plugin_host::sdk::NetworkRequest>, ToolError> {
    let mut requests = Vec::new();
    for spec in specs {
        let matches = extract_jsonpath_values(input, spec.jsonpath.as_str())?;
        if matches.is_empty() {
            if let Some(target) = spec.fallback.as_ref() {
                requests.push(agena_plugin_host::sdk::NetworkRequest {
                    target: target.clone(),
                });
                continue;
            }
            if spec.optional {
                continue;
            }
            return Err(ToolError::invalid_input(format!(
                "missing required input network '{}'",
                spec.jsonpath
            )));
        }
        for value in matches {
            let Some(target) = value.as_str() else {
                return Err(ToolError::invalid_input(format!(
                    "input network '{}' must resolve to a string",
                    spec.jsonpath
                )));
            };
            requests.push(agena_plugin_host::sdk::NetworkRequest {
                target: target.to_string(),
            });
        }
    }
    Ok(requests)
}

pub(super) fn extract_jsonpath_values<'a>(
    input: &'a serde_json::Value,
    jsonpath: &str,
) -> Result<Vec<&'a serde_json::Value>, ToolError> {
    let segments = parse_input_jsonpath(jsonpath)?;
    let mut current = vec![input];
    for segment in segments {
        let mut next = Vec::new();
        for value in current {
            match segment {
                InputJsonPathSegment::Key(ref key) => {
                    if let Some(object) = value.as_object()
                        && let Some(child) = object.get(key.as_str())
                    {
                        next.push(child);
                    }
                }
                InputJsonPathSegment::ArrayAll => {
                    if let Some(items) = value.as_array() {
                        next.extend(items.iter());
                    }
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    Ok(current)
}

pub(super) fn parse_input_jsonpath(jsonpath: &str) -> Result<Vec<InputJsonPathSegment>, ToolError> {
    if jsonpath == "$" {
        return Ok(Vec::new());
    }
    let Some(mut rest) = jsonpath.strip_prefix("$.") else {
        return Err(ToolError::invalid_input(format!(
            "unsupported input path jsonpath '{jsonpath}'"
        )));
    };

    let mut segments = Vec::new();
    while !rest.is_empty() {
        let key_end = rest.find(['.', '[']).unwrap_or(rest.len());
        let key = &rest[..key_end];
        if key.is_empty() {
            return Err(ToolError::invalid_input(format!(
                "unsupported input path jsonpath '{jsonpath}'"
            )));
        }
        segments.push(InputJsonPathSegment::Key(key.to_string()));
        rest = &rest[key_end..];

        while let Some(tail) = rest.strip_prefix("[*]") {
            segments.push(InputJsonPathSegment::ArrayAll);
            rest = tail;
        }

        if rest.is_empty() {
            break;
        }
        let Some(tail) = rest.strip_prefix('.') else {
            return Err(ToolError::invalid_input(format!(
                "unsupported input path jsonpath '{jsonpath}'"
            )));
        };
        rest = tail;
    }

    Ok(segments)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum InputJsonPathSegment {
    Key(String),
    ArrayAll,
}
use super::{
    AccessKind, Path, PathBuf, RegisteredTool, SdkInputNetworkSpec, SdkInputPathSpec, SdkPathKind,
    StructuredObject, ToolError, ToolInvocation, ToolOutput, ToolPayloadInput, shell_tools,
};
use agena_domain::{FilesystemEffects, PluginInvocation};
use agena_tool::ApplyPatchExecution;
