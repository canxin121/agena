pub(crate) fn normalize_path_for_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Resolve every existing component before a path is both authorized and used.
/// For a new output path, canonicalize the nearest existing parent and append
/// the missing suffix. This closes ordinary workspace-symlink escapes while
/// retaining support for creating new files.
pub(super) fn canonicalize_path_for_execution(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(&current) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return resolved;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = current.file_name().map(OsString::from) else {
                    return path.to_path_buf();
                };
                let Some(parent) = current.parent() else {
                    return path.to_path_buf();
                };
                missing.push(name);
                current = parent.to_path_buf();
            }
            Err(_) => return path.to_path_buf(),
        }
    }
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

pub(super) fn model_output_boundary_context(execution: &ToolInvocationExecution) -> String {
    let summary = execution.summary();
    let output_text = summary.output_text.as_str();
    let payload_text = execution
        .output
        .to_json_payload()
        .and_then(|payload| serde_json::to_string_pretty(&payload).ok())
        .unwrap_or_default();

    if payload_text.len() > output_text.len() {
        payload_text
    } else {
        output_text.to_string()
    }
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

/// The text preview alone is not enough: provider wire serialization also
/// includes `ToolOutput.payload`.  Bound that payload independently so a
/// giant JSON result cannot bypass the textual model-output limit.  We retain
/// concise structured facts (for example `changes` from `apply_patch`) and
/// leave the full, lossless result at the managed output path.
pub(super) fn compact_tool_output_payload_for_model(
    output: &mut ToolOutput,
    full_output_path: &str,
    original_bytes: usize,
) -> Result<(), ToolError> {
    let Some(payload) = output.to_json_payload() else {
        return Ok(());
    };
    let serialized = serde_json::to_string(&payload)
        .map_err(|error| ToolError::plugin(format!("encode tool output payload: {error}")))?;
    if serialized.len() <= TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES {
        return Ok(());
    }

    let compacted = compact_json_for_model(&payload, 0);
    let compacted_serialized = serde_json::to_string(&compacted).map_err(|error| {
        ToolError::plugin(format!("encode compact tool output payload: {error}"))
    })?;
    let payload = if compacted_serialized.len() <= TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES {
        compacted
    } else {
        serde_json::json!({
            "truncated": true,
            "full_output_path": full_output_path,
            "original_bytes": original_bytes,
        })
    };
    let managed_outputs = output.managed_outputs.clone();
    let truncated = output.truncated;
    let mut compact_output =
        ToolOutput::from_json_payload(Some(&payload)).map_err(ToolError::invalid_input)?;
    compact_output.managed_outputs = managed_outputs;
    compact_output.truncated = truncated;
    *output = compact_output;
    Ok(())
}

pub(super) fn compact_json_for_model(value: &serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= TOOL_MODEL_STRUCTURED_MAX_DEPTH {
        return serde_json::Value::String("[nested value omitted]".to_string());
    }
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            value.clone()
        }
        serde_json::Value::String(text) => {
            if text.len() <= TOOL_MODEL_STRUCTURED_STRING_MAX_BYTES {
                value.clone()
            } else {
                serde_json::Value::String(format!(
                    "{}… [string truncated; full output persisted]",
                    truncate_to_utf8_bytes(text, TOOL_MODEL_STRUCTURED_STRING_MAX_BYTES)
                ))
            }
        }
        serde_json::Value::Array(items) => {
            let mut compacted = items
                .iter()
                .take(TOOL_MODEL_STRUCTURED_MAX_ITEMS)
                .map(|item| compact_json_for_model(item, depth + 1))
                .collect::<Vec<_>>();
            let omitted = items.len().saturating_sub(compacted.len());
            if omitted > 0 {
                compacted.push(serde_json::Value::String(format!(
                    "[{omitted} array items omitted; full output persisted]"
                )));
            }
            serde_json::Value::Array(compacted)
        }
        serde_json::Value::Object(object) => {
            let mut compacted = serde_json::Map::new();
            // Keep the small, high-signal fields first even when an object has
            // many keys. This preserves patch file lists and common counters.
            for key in [
                "changes", "count", "matches", "query", "path", "url", "status", "error", "results",
            ] {
                if let Some(value) = object.get(key) {
                    compacted.insert(key.to_string(), compact_json_for_model(value, depth + 1));
                }
            }
            for (key, value) in object {
                if compacted.len() >= TOOL_MODEL_STRUCTURED_MAX_FIELDS {
                    break;
                }
                compacted
                    .entry(key.clone())
                    .or_insert_with(|| compact_json_for_model(value, depth + 1));
            }
            if object.len() > compacted.len() {
                compacted.insert(
                    "_agena_omitted_fields".to_string(),
                    serde_json::Value::from(object.len().saturating_sub(compacted.len())),
                );
            }
            compacted.insert(
                "_agena_truncated".to_string(),
                serde_json::Value::Bool(true),
            );
            serde_json::Value::Object(compacted)
        }
    }
}

pub(super) fn persist_tool_result_output(
    workspace_root: &Path,
    model_tool_name: &str,
    call_id: i64,
    output_text: &str,
) -> Result<Option<PathBuf>, ToolError> {
    if output_text.is_empty() {
        return Ok(None);
    }

    let dir = workspace_root.join(".agena").join("tool-results");
    fs::create_dir_all(&dir)?;
    let digest = blake3::hash(output_text.as_bytes()).to_hex().to_string();
    let short_digest = digest.get(..12).unwrap_or(digest.as_str());
    let safe_tool = tool_result_file_stem(model_tool_name);
    let call_part = if call_id >= 0 {
        call_id.to_string()
    } else {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().to_string())
            .unwrap_or_else(|_| "synthetic".to_string())
    };
    let path = dir.join(format!("{call_part}-{safe_tool}-{short_digest}.txt"));
    let mut file = fs::File::create(&path)?;
    file.write_all(output_text.as_bytes())?;
    Ok(Some(path))
}

pub(super) fn tool_result_file_stem(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "tool".to_string();
    }
    let mut stem = String::with_capacity(trimmed.len());
    let mut previous_was_separator = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            stem.push(ch);
            previous_was_separator = false;
        } else if !previous_was_separator {
            stem.push('_');
            previous_was_separator = true;
        }
    }
    let stem = stem.trim_matches('_');
    if stem.is_empty() {
        "tool".to_string()
    } else {
        stem.to_string()
    }
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
    effects: &[FilesystemEffect],
) -> Result<(), ToolError> {
    shell_tools::validate_declared_filesystem_effects(tool_name, command, effects)
}

pub(super) fn shell_command_from_invocation(invocation: &ToolInvocation) -> Option<String> {
    if let Some(payload) = ToolPayloadInput::from_invocation(invocation) {
        let command = match payload {
            ToolPayloadInput::Shell(crate::message::ShellToolInput::Run { command, .. }) => {
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
) -> Result<Option<Vec<FilesystemEffect>>, ToolError> {
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
    let operation_id = payload.get("operation_id")?.as_str()?.to_string();
    let changes: Vec<agena_domain::FileChangeRecord> =
        serde_json::from_value(payload.get("changes")?.clone()).ok()?;
    let before_hash = payload
        .get("before_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let after_hash = payload
        .get("after_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let inverse_patch = payload.get("inverse_patch")?.as_str()?.to_string();
    let diff = payload
        .get("diff")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let progress = serde_json::from_value(
        payload
            .get("progress")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .ok()?;
    Some(ApplyPatchExecution {
        operation_id,
        files: changes
            .into_iter()
            .map(|change| AppliedFileChange {
                path: change.path,
                kind: match change.kind {
                    agena_domain::FileChangeKind::Added => PatchOpKind::Add,
                    agena_domain::FileChangeKind::Updated => PatchOpKind::Update,
                    agena_domain::FileChangeKind::Deleted => PatchOpKind::Delete,
                    agena_domain::FileChangeKind::Moved => PatchOpKind::Move,
                },
                from_path: change.from_path,
            })
            .collect(),
        before_hash,
        after_hash,
        inverse_patch,
        diff,
        progress,
    })
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
    AccessKind, OsString, Path, PathBuf, RegisteredTool, SdkInputNetworkSpec, SdkInputPathSpec,
    SdkPathKind, StructuredObject, TOOL_MODEL_STRUCTURED_MAX_DEPTH,
    TOOL_MODEL_STRUCTURED_MAX_FIELDS, TOOL_MODEL_STRUCTURED_MAX_ITEMS,
    TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES, TOOL_MODEL_STRUCTURED_STRING_MAX_BYTES, ToolError,
    ToolInvocation, ToolInvocationExecution, ToolOutput, ToolPayloadInput, Write, fs, shell_tools,
};
use agena_domain::{FilesystemEffect, PluginInvocation};
use agena_tool::{AppliedFileChange, ApplyPatchExecution, PatchOpKind};
