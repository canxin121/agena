use clap::CommandFactory;

use super::{
    AgenaCli, AppError, ApplyPatchExecution, BTreeMap, CompletionArgs, DateTime,
    DebugSessionOutput, HashSet, OutputFormat, Path, PathBuf, PluginLogsOutput,
    PluginValidateOutput, PluginValidationMessage, PluginValidationMessages, Serialize, Utc, fs,
};

pub(super) fn render_completion_command(args: CompletionArgs) -> Result<String, AppError> {
    let mut command = AgenaCli::command();
    let mut buffer = Vec::new();
    clap_complete::generate(args.shell, &mut command, "agena", &mut buffer);
    String::from_utf8(buffer)
        .map_err(|err| AppError::Internal(format!("completion output was not utf-8: {err}")))
}

pub(super) fn render_serialized<T>(format: OutputFormat, value: &T) -> Result<String, AppError>
where
    T: Serialize,
{
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(value)?),
    }
}

pub(super) fn format_apply_output(execution: &ApplyPatchExecution) -> String {
    let mut output = format!("applied patch: {}", execution.operation_id);
    for file in &execution.files {
        output.push_str(&format!("\n- {:?} {}", file.kind, file.path));
    }
    output
}

pub(super) fn format_debug_session_output(output: &DebugSessionOutput) -> String {
    let mut rendered = format!(
        "session {}: {}\nstatus: {:?}\nmessages: {}",
        output.session.id,
        output.session.title,
        output.session.status,
        output.messages.len()
    );
    for message in &output.messages {
        rendered.push_str(&format!(
            "\n\n[{} #{} {}]\n{}",
            message.role, message.id, message.state, message.text
        ));
    }
    rendered
}

pub(super) fn format_plugin_logs_output(output: &PluginLogsOutput) -> String {
    if output.logs.is_empty() {
        return format!("plugin {} has no retained logs", output.plugin_id);
    }
    output
        .logs
        .iter()
        .map(|log| {
            let timestamp = DateTime::<Utc>::from_timestamp_millis(log.timestamp_ms)
                .map(|ts| ts.to_rfc3339())
                .unwrap_or_else(|| log.timestamp_ms.to_string());
            let mut line = format!(
                "[{}] #{} {} {} {}",
                timestamp, log.seq, log.level, log.source, log.message
            );
            if !log.fields.is_null() {
                line.push(' ');
                line.push_str(&log.fields.to_string());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn render_plugin_validate_output(
    format: OutputFormat,
    output: &PluginValidateOutput,
) -> Result<String, AppError> {
    render_serialized(format, output)
}

pub(super) fn validate_plugin_target(
    path: &Path,
    strict: bool,
) -> Result<PluginValidateOutput, AppError> {
    let path = resolve_plugin_validate_path(path)?;
    let raw = fs::read_to_string(&path)?;
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let mut target_kind = "unknown".to_string();
    let mut manifest_hash = None;
    let mut messages: PluginValidationMessages = (Vec::new(), Vec::new());
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

    if looks_like_plugin_manifest(&value) {
        target_kind = "manifest".to_string();
        validate_plugin_manifest_value("$", &value, &mut manifest_hash, &mut messages);
    } else if value.get("package").is_some() {
        target_kind = "configured_plugin".to_string();
        validate_configured_plugin_value("$", &value, base_dir, &BTreeMap::new(), &mut messages);
    } else if let Some(plugin_list) = value.pointer("/plugins/list").and_then(|v| v.as_object()) {
        target_kind = "agena_config".to_string();
        let trusted_keys = value
            .pointer("/plugins/host/trusted_keys")
            .cloned()
            .and_then(|v| serde_json::from_value::<BTreeMap<String, String>>(v).ok())
            .unwrap_or_default();
        if plugin_list.is_empty() {
            push_warning(
                &mut messages,
                "config.plugins.empty",
                "plugins.list is empty",
                Some("$.plugins.list"),
            );
        }
        for (plugin_id, plugin_value) in plugin_list {
            validate_configured_plugin_value(
                &format!("$.plugins.list.{plugin_id}"),
                plugin_value,
                base_dir,
                &trusted_keys,
                &mut messages,
            );
        }
    } else {
        push_error(
            &mut messages,
            "target.unsupported",
            "expected a plugin manifest, configured plugin object, or agena config with plugins.list",
            Some("$"),
        );
    }

    if strict && !messages.1.is_empty() {
        for warning in messages.1.clone() {
            messages.0.push(PluginValidationMessage {
                code: format!("strict.{}", warning.code),
                message: format!("warning treated as error: {}", warning.message),
                path: warning.path,
            });
        }
    }
    let errors = messages.0;
    let warnings = messages.1;
    Ok(PluginValidateOutput {
        path: path.display().to_string(),
        target_kind,
        ok: errors.is_empty(),
        manifest_hash,
        errors,
        warnings,
    })
}

pub(super) fn resolve_plugin_validate_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if path.is_dir() {
        for candidate in [
            path.join(".agena-plugin").join("plugin.json"),
            path.join("plugin.json"),
            path.join("manifest.json"),
        ] {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(AppError::Config(format!(
        "plugin validate target not found: {}",
        path.display()
    )))
}

pub(super) fn looks_like_plugin_manifest(value: &serde_json::Value) -> bool {
    value.get("schema_version").is_some()
        || value.get("tools").is_some()
        || value.get("transports").is_some()
        || (value.get("name").is_some() && value.get("version").is_some())
}

pub(super) fn validate_plugin_manifest_value(
    path: &str,
    value: &serde_json::Value,
    manifest_hash: &mut Option<String>,
    output: &mut PluginValidationMessages,
) {
    check_object_keys(
        value,
        path,
        &[
            "schema_version",
            "name",
            "version",
            "description",
            "summary",
            "help",
            "tool_description_mode",
            "ui_display_mode",
            "authors",
            "transports",
            "hooks",
            "tools",
            "commands",
            "plugin_capabilities",
            "ui",
            "config_schema",
            "config_schema_i18n",
        ],
        "manifest.unknown_field",
        output,
    );
    warn_marketplace_fields(value, path, output);
    validate_raw_hook_array(value.get("hooks"), &format!("{path}.hooks"), output);

    let manifest: agena_plugin_host::PluginManifest = match serde_json::from_value(value.clone()) {
        Ok(manifest) => manifest,
        Err(err) => {
            push_error(
                output,
                "manifest.schema",
                format!("manifest does not match plugin manifest schema: {err}"),
                Some(path),
            );
            return;
        }
    };

    *manifest_hash = serde_json::to_vec(&manifest)
        .ok()
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string());

    if manifest.schema_version != 1 {
        push_warning(
            output,
            "manifest.schema_version",
            format!(
                "schema_version {} is not the current schema_version 1",
                manifest.schema_version
            ),
            Some(format!("{path}.schema_version")),
        );
    }
    if manifest.namespace.trim().is_empty() {
        push_error(
            output,
            "manifest.namespace.empty",
            "manifest namespace must not be empty",
            Some(format!("{path}.namespace")),
        );
    }
    if manifest.name.trim().is_empty() {
        push_error(
            output,
            "manifest.name.empty",
            "manifest name must not be empty",
            Some(format!("{path}.name")),
        );
    }
    if manifest.transports.is_empty() {
        push_warning(
            output,
            "manifest.transports.empty",
            "manifest declares no transport kind",
            Some(format!("{path}.transports")),
        );
    }

    if let Some(tools) = value.get("tools").and_then(|v| v.as_array()) {
        for (idx, tool_value) in tools.iter().enumerate() {
            validate_tool_manifest_value(
                &manifest.namespace,
                &manifest.name,
                &manifest.tools.get(idx),
                tool_value,
                &format!("{path}.tools[{idx}]"),
                output,
            );
        }
    }
    validate_tool_name_collisions(&manifest, path, output);
    validate_manifest_ui_actions(&manifest, path, output);

    if let Some(schema) = manifest.config_schema.as_ref() {
        validate_schema_defaults(&format!("{path}.config_schema"), schema, output);
    }
    for (locale, schema) in &manifest.config_schema_i18n {
        validate_schema_defaults(
            &format!("{path}.config_schema_i18n.{locale}"),
            schema,
            output,
        );
    }

    if !manifest.tools.is_empty()
        && !manifest.hooks.intersects(
            agena_plugin_host::HookSubscription::TOOL_INVOKE
                | agena_plugin_host::HookSubscription::TOOL_INVOKE_STREAM,
        )
    {
        push_warning(
            output,
            "manifest.hooks.tool_invoke_missing",
            "manifest declares tools but does not subscribe to tool.invoke or tool.invoke.stream",
            Some(format!("{path}.hooks")),
        );
    }
}

pub(super) fn validate_tool_manifest_value(
    plugin_namespace: &str,
    plugin_name: &str,
    parsed_tool: &Option<&agena_plugin_host::ToolDefinition>,
    value: &serde_json::Value,
    path: &str,
    output: &mut PluginValidationMessages,
) {
    check_object_keys(
        value,
        path,
        &[
            "name",
            "aliases",
            "contract",
            "model",
            "docs",
            "runtime",
            "permissions",
            "display",
            "capabilities",
        ],
        "tool.unknown_field",
        output,
    );
    if let Some(contract) = value.get("contract") {
        check_object_keys(
            contract,
            &format!("{path}.contract"),
            &["input_schema", "output_schema", "strict"],
            "tool.contract.unknown_field",
            output,
        );
    }
    if let Some(model) = value.get("model") {
        check_object_keys(
            model,
            &format!("{path}.model"),
            &["description", "examples"],
            "tool.model.unknown_field",
            output,
        );
    }
    if let Some(docs) = value.get("docs") {
        check_object_keys(
            docs,
            &format!("{path}.docs"),
            &["before_help", "after_help", "summary", "help"],
            "tool.docs.unknown_field",
            output,
        );
    }
    if let Some(runtime) = value.get("runtime") {
        check_object_keys(
            runtime,
            &format!("{path}.runtime"),
            &["concurrency_safe", "streaming", "result_policy"],
            "tool.runtime.unknown_field",
            output,
        );
    }
    if let Some(policy) = value.pointer("/runtime/result_policy") {
        check_object_keys(
            policy,
            &format!("{path}.runtime.result_policy"),
            &[
                "max_model_chars",
                "preview_lines",
                "persist_large_output",
                "ui_render_kind",
            ],
            "tool.result_policy.unknown_field",
            output,
        );
    }
    if let Some(permissions) = value.get("permissions") {
        check_object_keys(
            permissions,
            &format!("{path}.permissions"),
            &[
                "input_paths",
                "input_networks",
                "path_access",
                "network_access",
                "tags",
            ],
            "tool.permissions.unknown_field",
            output,
        );
    }
    if let Some(display) = value.get("display") {
        check_object_keys(
            display,
            &format!("{path}.display"),
            &["description_mode", "ui_display_mode"],
            "tool.display.unknown_field",
            output,
        );
    }
    if let Some(tool) = parsed_tool.as_ref() {
        validate_tool_segment(
            plugin_namespace,
            plugin_name,
            tool.name.as_str(),
            &format!("{path}.name"),
            output,
        );
        for (idx, spec) in tool.permissions.path_access.iter().enumerate() {
            validate_no_parent_path(
                spec.path.as_str(),
                &format!("{path}.path_access[{idx}].path"),
                output,
            );
        }
    }
}

pub(super) fn validate_tool_segment(
    _plugin_namespace: &str,
    _plugin_name: &str,
    tool_name: &str,
    path: &str,
    output: &mut PluginValidationMessages,
) {
    if tool_name.trim().is_empty() {
        push_error(
            output,
            "tool.name.empty",
            "tool name must not be empty",
            Some(path),
        );
    }
}

pub(super) fn validate_tool_name_collisions(
    manifest: &agena_plugin_host::PluginManifest,
    path: &str,
    output: &mut PluginValidationMessages,
) {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (idx, tool) in manifest.tools.iter().enumerate() {
        let raw_name = tool.name.as_str();
        let location = format!("{path}.tools[{idx}].name");
        if let Some(existing) = seen.insert(raw_name.to_string(), location.clone()) {
            push_error(
                output,
                "tool.name.collision",
                format!("duplicate tool name `{raw_name}`, colliding with {existing}"),
                Some(location),
            );
        }
    }
}

pub(super) fn validate_manifest_ui_actions(
    manifest: &agena_plugin_host::PluginManifest,
    path: &str,
    output: &mut PluginValidationMessages,
) {
    let known_tools = manifest
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    for (idx, command) in manifest.commands.iter().enumerate() {
        validate_ui_action_tool(
            &command.action,
            &known_tools,
            &format!("{path}.commands[{idx}].action"),
            output,
        );
    }
    for (idx, control) in manifest.ui.studio.controls.iter().enumerate() {
        validate_ui_action_tool(
            &control.action,
            &known_tools,
            &format!("{path}.ui.studio.controls[{idx}].action"),
            output,
        );
    }
}

pub(super) fn validate_ui_action_tool(
    action: &agena_plugin_host::PluginUiAction,
    known_tools: &HashSet<&str>,
    path: &str,
    output: &mut PluginValidationMessages,
) {
    if let agena_plugin_host::PluginUiAction::InvokeTool { tool, .. } = action {
        if tool.contains('/') {
            push_error(
                output,
                "ui.action.tool.invalid",
                "UI action tool must use the local tool name or model-visible dotted tool name, not plugin/tool",
                Some(format!("{path}.tool")),
            );
        }
        if !known_tools.contains(tool.as_str()) && !tool.contains("__") && !tool.contains('.') {
            push_warning(
                output,
                "ui.action.tool.unknown",
                format!("UI action references unknown local tool `{tool}`"),
                Some(format!("{path}.tool")),
            );
        }
    }
}

pub(super) fn validate_configured_plugin_value(
    path: &str,
    value: &serde_json::Value,
    base_dir: &Path,
    trusted_keys: &BTreeMap<String, String>,
    output: &mut PluginValidationMessages,
) {
    let configured: agena_plugin_host::ConfiguredPlugin =
        match serde_json::from_value(value.clone()) {
            Ok(configured) => configured,
            Err(err) => {
                push_error(
                    output,
                    "config_plugin.schema",
                    format!("configured plugin does not match schema: {err}"),
                    Some(path),
                );
                return;
            }
        };

    match &configured.package {
        agena_plugin_host::PluginPackage::Static {} => {}
        agena_plugin_host::PluginPackage::Cdylib {
            path: package_path,
            sha256,
            signature,
        } => {
            let resolved = resolve_config_path(base_dir, package_path);
            validate_no_parent_path(
                package_path.to_string_lossy().as_ref(),
                &format!("{path}.package.path"),
                output,
            );
            validate_existing_file(&resolved, &format!("{path}.package.path"), output);
            validate_sha256_if_present(
                &resolved,
                sha256.as_deref(),
                &format!("{path}.package.sha256"),
                output,
            );
            validate_signature_if_present(
                &resolved,
                signature.as_ref(),
                trusted_keys,
                &format!("{path}.package.signature"),
                output,
            );
        }
        agena_plugin_host::PluginPackage::Stdio {
            command,
            cwd,
            sha256,
            ..
        } => {
            if let Some(cwd) = cwd {
                validate_no_parent_path(
                    cwd.to_string_lossy().as_ref(),
                    &format!("{path}.package.cwd"),
                    output,
                );
                let resolved_cwd = resolve_config_path(base_dir, cwd);
                if !resolved_cwd.is_dir() {
                    push_error(
                        output,
                        "transport.cwd.missing",
                        format!(
                            "stdio cwd does not exist or is not a directory: {}",
                            resolved_cwd.display()
                        ),
                        Some(format!("{path}.package.cwd")),
                    );
                }
            }
            let resolved_command = resolve_command_path(command, cwd.as_deref(), base_dir);
            match resolved_command {
                Some(command_path) => {
                    validate_sha256_if_present(
                        &command_path,
                        sha256.as_deref(),
                        &format!("{path}.package.sha256"),
                        output,
                    );
                }
                None => push_error(
                    output,
                    "transport.command.not_found",
                    format!("stdio command is not executable or not found on PATH: {command}"),
                    Some(format!("{path}.package.command")),
                ),
            }
        }
        agena_plugin_host::PluginPackage::Http { url, .. } => {
            if !matches!(url.scheme(), "http" | "https") {
                push_error(
                    output,
                    "transport.http.scheme",
                    format!("unsupported http plugin URL scheme `{}`", url.scheme()),
                    Some(format!("{path}.package.url")),
                );
            }
        }
        agena_plugin_host::PluginPackage::Wasm {
            path: wasm_path,
            sha256,
        } => {
            let resolved = resolve_config_path(base_dir, wasm_path);
            validate_no_parent_path(
                wasm_path.to_string_lossy().as_ref(),
                &format!("{path}.package.path"),
                output,
            );
            validate_existing_file(&resolved, &format!("{path}.package.path"), output);
            validate_sha256_if_present(
                &resolved,
                sha256.as_deref(),
                &format!("{path}.package.sha256"),
                output,
            );
        }
    }
}

pub(super) fn validate_raw_hook_array(
    hooks: Option<&serde_json::Value>,
    path: &str,
    output: &mut PluginValidationMessages,
) {
    let Some(hooks) = hooks else {
        return;
    };
    let Some(items) = hooks.as_array() else {
        push_error(
            output,
            "hooks.schema",
            "hooks must be an array of hook names",
            Some(path),
        );
        return;
    };
    for (idx, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{idx}]");
        let Some(name) = item.as_str() else {
            push_error(
                output,
                "hooks.schema",
                "hook subscription must be a string",
                Some(item_path),
            );
            continue;
        };
        if agena_plugin_host::HookSubscription::for_name(name).is_none() {
            push_error(
                output,
                "hooks.unknown",
                format!("unknown hook subscription `{name}`"),
                Some(item_path),
            );
        }
    }
}

pub(super) fn validate_schema_defaults(
    path: &str,
    schema: &serde_json::Value,
    output: &mut PluginValidationMessages,
) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(default_value) = object.get("default") {
        validate_default_matches_schema(path, schema, default_value, output);
    }
    for key in ["properties", "$defs", "definitions"] {
        if let Some(children) = object.get(key).and_then(|v| v.as_object()) {
            for (name, child) in children {
                validate_schema_defaults(&format!("{path}.{key}.{name}"), child, output);
            }
        }
    }
    if let Some(items) = object.get("items") {
        validate_schema_defaults(&format!("{path}.items"), items, output);
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(items) = object.get(key).and_then(|v| v.as_array()) {
            for (idx, child) in items.iter().enumerate() {
                validate_schema_defaults(&format!("{path}.{key}[{idx}]"), child, output);
            }
        }
    }
}

pub(super) fn validate_default_matches_schema(
    path: &str,
    schema: &serde_json::Value,
    default_value: &serde_json::Value,
    output: &mut PluginValidationMessages,
) {
    if let Some(enum_values) = schema.get("enum").and_then(|v| v.as_array())
        && !enum_values.iter().any(|value| value == default_value)
    {
        push_error(
            output,
            "config_schema.default.enum",
            "default value is not present in enum",
            Some(format!("{path}.default")),
        );
    }
    if let Some(type_names) = schema_type_names(schema)
        && !type_names
            .iter()
            .any(|type_name| json_value_matches_type(default_value, type_name))
    {
        push_error(
            output,
            "config_schema.default.type",
            format!(
                "default value does not match schema type {}",
                type_names.join("|")
            ),
            Some(format!("{path}.default")),
        );
    }
    if let Some(required) = schema.get("required").and_then(|v| v.as_array())
        && let Some(default_object) = default_value.as_object()
    {
        for required_name in required.iter().filter_map(|v| v.as_str()) {
            if !default_object.contains_key(required_name) {
                push_error(
                    output,
                    "config_schema.default.required",
                    format!("default object is missing required field `{required_name}`"),
                    Some(format!("{path}.default")),
                );
            }
        }
    }
    if let (Some(properties), Some(default_object)) = (
        schema.get("properties").and_then(|v| v.as_object()),
        default_value.as_object(),
    ) {
        for (name, property_schema) in properties {
            if let Some(child_default) = default_object.get(name) {
                validate_default_matches_schema(
                    &format!("{path}.properties.{name}"),
                    property_schema,
                    child_default,
                    output,
                );
            }
        }
    }
}

pub(super) fn schema_type_names(schema: &serde_json::Value) -> Option<Vec<String>> {
    let value = schema.get("type")?;
    if let Some(name) = value.as_str() {
        return Some(vec![name.to_string()]);
    }
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
    )
}

pub(super) fn json_value_matches_type(value: &serde_json::Value, type_name: &str) -> bool {
    match type_name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        _ => true,
    }
}

pub(super) fn validate_no_parent_path(
    path_value: &str,
    path: &str,
    output: &mut PluginValidationMessages,
) {
    if Path::new(path_value)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        push_error(
            output,
            "path.traversal",
            format!("path must not contain `..`: {path_value}"),
            Some(path),
        );
    }
}

pub(super) fn validate_existing_file(
    path: &Path,
    json_path: &str,
    output: &mut PluginValidationMessages,
) {
    if !path.is_file() {
        push_error(
            output,
            "transport.file.not_found",
            format!("transport file does not exist: {}", path.display()),
            Some(json_path),
        );
    }
}

pub(super) fn validate_sha256_if_present(
    path: &Path,
    expected: Option<&str>,
    json_path: &str,
    output: &mut PluginValidationMessages,
) {
    let Some(expected) = expected else {
        push_warning(
            output,
            "transport.sha256.missing",
            "transport artifact has no sha256 pin",
            Some(json_path),
        );
        return;
    };
    if !path.is_file() {
        return;
    }
    match sha256_hex_file(path) {
        Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
        Ok(actual) => push_error(
            output,
            "transport.sha256.mismatch",
            format!("sha256 mismatch: expected {expected}, got {actual}"),
            Some(json_path),
        ),
        Err(err) => push_error(
            output,
            "transport.sha256.read_failed",
            format!("failed to compute sha256: {err}"),
            Some(json_path),
        ),
    }
}

pub(super) fn validate_signature_if_present(
    path: &Path,
    signature: Option<&agena_plugin_host::PluginSignature>,
    trusted_keys: &BTreeMap<String, String>,
    json_path: &str,
    output: &mut PluginValidationMessages,
) {
    let Some(signature) = signature else {
        push_warning(
            output,
            "transport.signature.missing",
            "transport artifact has no signature",
            Some(json_path),
        );
        return;
    };
    if !trusted_keys.contains_key(&signature.key_id) {
        push_error(
            output,
            "transport.signature.untrusted_key",
            format!(
                "signature key `{}` is not configured as trusted",
                signature.key_id
            ),
            Some(format!("{json_path}.key_id")),
        );
        return;
    }
    #[cfg(feature = "plugin-signing")]
    {
        if path.is_file()
            && let Err(err) = agena_plugin_host::verify_signature(path, signature, trusted_keys)
        {
            push_error(output, "transport.signature.invalid", err, Some(json_path));
        }
    }
    #[cfg(not(feature = "plugin-signing"))]
    {
        let _ = path;
        push_warning(
            output,
            "transport.signature.not_verified",
            "signature is present but this binary was built without plugin-signing",
            Some(json_path),
        );
    }
}

pub(super) fn sha256_hex_file(path: &Path) -> Result<String, std::io::Error> {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(hex::encode(digest))
}

pub(super) fn resolve_config_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

pub(super) fn resolve_command_path(
    command: &str,
    cwd: Option<&Path>,
    base_dir: &Path,
) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() > 1 || command.contains(std::path::MAIN_SEPARATOR) {
        let base = cwd
            .map(|cwd| resolve_config_path(base_dir, cwd))
            .unwrap_or_else(|| base_dir.to_path_buf());
        let resolved = if command_path.is_absolute() {
            command_path.to_path_buf()
        } else {
            base.join(command_path)
        };
        return is_executable_file(&resolved).then_some(resolved);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(command))
            .find(|candidate| is_executable_file(candidate))
    })
}

pub(super) fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(super) fn check_object_keys(
    value: &serde_json::Value,
    path: &str,
    allowed: &[&str],
    code: &str,
    output: &mut PluginValidationMessages,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            push_error(
                output,
                code,
                format!("unknown field `{key}`"),
                Some(format!("{path}.{key}")),
            );
        }
    }
}

pub(super) fn warn_marketplace_fields(
    value: &serde_json::Value,
    path: &str,
    output: &mut PluginValidationMessages,
) {
    const MARKETPLACE_FIELDS: &[&str] = &[
        "id",
        "homepage",
        "repository",
        "license",
        "category",
        "versions",
        "artifact",
        "archive",
        "sha256",
        "signature",
        "registry",
        "install",
        "marketplace",
    ];
    let Some(object) = value.as_object() else {
        return;
    };
    for field in MARKETPLACE_FIELDS {
        if object.contains_key(*field) {
            push_warning(
                output,
                "manifest.marketplace_field",
                format!("marketplace field `{field}` does not belong in plugin manifest"),
                Some(format!("{path}.{field}")),
            );
        }
    }
}

pub(super) fn push_error(
    output: &mut PluginValidationMessages,
    code: impl Into<String>,
    message: impl Into<String>,
    path: Option<impl Into<String>>,
) {
    output.0.push(PluginValidationMessage {
        code: code.into(),
        message: message.into(),
        path: path.map(Into::into),
    });
}

pub(super) fn push_warning(
    output: &mut PluginValidationMessages,
    code: impl Into<String>,
    message: impl Into<String>,
    path: Option<impl Into<String>>,
) {
    output.1.push(PluginValidationMessage {
        code: code.into(),
        message: message.into(),
        path: path.map(Into::into),
    });
}
