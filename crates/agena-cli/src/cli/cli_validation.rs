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
        "session {}: {}\nstatus: {:?}\nruns: {}",
        output.session.id,
        output.session.title,
        output.session.status,
        output.runs.len()
    );
    for run in &output.runs {
        rendered.push_str(&format!(
            "\n\n[{} #{} {}]\n{}",
            run.role, run.id, run.state, run.text
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
    if path.file_name().is_some_and(|name| {
        name == agena_plugin_marketplace::project::AGENA_PROJECT_MANIFEST_FILENAME
    }) {
        let mut messages: PluginValidationMessages = (Vec::new(), Vec::new());
        if let Err(error) = agena_plugin_marketplace::PluginProjectManifest::load(&path) {
            push_error(
                &mut messages,
                "project.invalid",
                error.to_string(),
                Some("$"),
            );
        }
        return finish_plugin_validation(&path, "plugin_project", None, messages, strict);
    }
    if path
        .file_name()
        .is_some_and(|name| name == agena_plugin_marketplace::AGENA_MARKETPLACE_PROJECT_FILENAME)
    {
        let mut messages: PluginValidationMessages = (Vec::new(), Vec::new());
        if let Err(error) = agena_plugin_marketplace::MarketplaceProjectManifest::load(&path) {
            push_error(
                &mut messages,
                "marketplace_project.invalid",
                error.to_string(),
                Some("$"),
            );
        }
        return finish_plugin_validation(&path, "marketplace_project", None, messages, strict);
    }
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let mut target_kind = "unknown".to_string();
    let mut manifest_hash = None;
    let mut messages: PluginValidationMessages = (Vec::new(), Vec::new());
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

    if value.get("plugins").is_some() && value.get("version").is_some() {
        target_kind = "marketplace_index".to_string();
        match serde_json::from_value::<agena_plugin_marketplace::RegistryIndex>(value.clone()) {
            Ok(index) => {
                if let Err(error) = index.validate() {
                    push_error(
                        &mut messages,
                        "marketplace.invalid",
                        error.to_string(),
                        Some("$"),
                    );
                }
            }
            Err(error) => push_error(
                &mut messages,
                "marketplace.decode",
                error.to_string(),
                Some("$"),
            ),
        }
    } else if value.get("artifacts").is_some() && value.get("id").is_some() {
        target_kind = "plugin_release".to_string();
        match serde_json::from_value::<agena_plugin_marketplace::PluginReleaseManifest>(
            value.clone(),
        ) {
            Ok(release) => {
                if let Err(error) = release.validate() {
                    push_error(
                        &mut messages,
                        "release.invalid",
                        error.to_string(),
                        Some("$"),
                    );
                }
            }
            Err(error) => push_error(
                &mut messages,
                "release.decode",
                error.to_string(),
                Some("$"),
            ),
        }
    } else if looks_like_plugin_manifest(&value) {
        target_kind = "manifest".to_string();
        validate_plugin_manifest_value("$", &value, &mut manifest_hash, &mut messages);
    } else if value.get("package").is_some() {
        target_kind = "configured_plugin".to_string();
        validate_configured_plugin_value("$", &value, base_dir, &BTreeMap::new(), &mut messages);
    } else if let Some(plugins_value) = value.get("plugins") {
        target_kind = "agena_config".to_string();
        let trusted_keys = match value.pointer("/plugins/host/trusted_keys").cloned() {
            Some(value) => match serde_json::from_value::<BTreeMap<String, String>>(value) {
                Ok(keys) => keys,
                Err(error) => {
                    push_error(
                        &mut messages,
                        "config.plugins.trusted_keys.decode",
                        agena_failure::diagnostic::format_error_chain_with_context(
                            "decode plugins.host.trusted_keys",
                            &error,
                        ),
                        Some("$.plugins.host.trusted_keys"),
                    );
                    BTreeMap::new()
                }
            },
            None => BTreeMap::new(),
        };
        match serde_json::from_value::<agena_plugin_host::PluginsConfig>(plugins_value.clone()) {
            Ok(plugins) => match plugins.resolved_profile_view() {
                Ok(resolution) => {
                    if resolution.list.is_empty() {
                        push_warning(
                            &mut messages,
                            "config.plugins.empty",
                            "resolved plugins.list is empty",
                            Some("$.plugins.list"),
                        );
                    }
                    for (plugin_id, configured) in &resolution.list {
                        let configured_value =
                            serde_json::to_value(configured).map_err(|error| {
                                AppError::Internal(format!(
                                    "failed to serialize resolved plugin `{plugin_id}`: {error}"
                                ))
                            })?;
                        validate_configured_plugin_value(
                            &format!("$.plugins.list.{plugin_id}"),
                            &configured_value,
                            base_dir,
                            &trusted_keys,
                            &mut messages,
                        );
                    }
                    match agena_plugin_host::plan_plugin_activation(&resolution.list) {
                        Ok(plan) => {
                            for block in plan.blocked.values() {
                                push_error(
                                    &mut messages,
                                    format!("config.plugin.activation.{}", block.code),
                                    block.message.clone(),
                                    Some(format!("$.plugins.list.{}.activation", block.plugin_id)),
                                );
                            }
                        }
                        Err(message) => push_error(
                            &mut messages,
                            "config.plugin.activation.invalid",
                            message,
                            Some("$.plugins.list"),
                        ),
                    }
                }
                Err(message) => push_error(
                    &mut messages,
                    "config.plugin.profile.invalid",
                    message,
                    Some("$.plugins.profiles"),
                ),
            },
            Err(error) => push_error(
                &mut messages,
                "config.plugins.decode",
                error.to_string(),
                Some("$.plugins"),
            ),
        }
    } else {
        push_error(
            &mut messages,
            "target.unsupported",
            "expected a plugin manifest, configured plugin object, or agena config with plugins.list",
            Some("$"),
        );
    }

    finish_plugin_validation(&path, &target_kind, manifest_hash, messages, strict)
}

fn finish_plugin_validation(
    path: &Path,
    target_kind: &str,
    manifest_hash: Option<String>,
    mut messages: PluginValidationMessages,
    strict: bool,
) -> Result<PluginValidateOutput, AppError> {
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
        target_kind: target_kind.to_string(),
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
            path.join(agena_plugin_marketplace::project::AGENA_PROJECT_MANIFEST_FILENAME),
            path.join(agena_plugin_marketplace::AGENA_MARKETPLACE_PROJECT_FILENAME),
            path.join(agena_plugin_marketplace::AGENA_RELEASE_MANIFEST_FILENAME),
            path.join(agena_plugin_marketplace::AGENA_MARKETPLACE_FILENAME),
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
            "namespace",
            "name",
            "version",
            "summary",
            "help",
            "authors",
            "transports",
            "hooks",
            "tools",
            "operations",
            "services",
            "activity_kinds",
            "tags",
            "skills",
            "surface",
            "settings",
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
    validate_manifest_operations(&manifest, path, output);
    if let Err(error) = manifest.services.validate() {
        push_error(
            output,
            "manifest.services.invalid",
            error.to_string(),
            Some(format!("{path}.services")),
        );
    }

    if let Some(settings) = manifest.settings.as_ref()
        && let Err(error) = settings.validate()
    {
        push_error(
            output,
            "manifest.settings.invalid",
            error.to_string(),
            Some(format!("{path}.settings")),
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

pub(super) fn validate_manifest_operations(
    manifest: &agena_plugin_host::PluginManifest,
    path: &str,
    output: &mut PluginValidationMessages,
) {
    use agena_plugin_host::sdk::PluginOperationTarget;

    let known_tools = manifest
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    let mut ids = HashSet::new();
    for (idx, operation) in manifest.operations.iter().enumerate() {
        let operation_path = format!("{path}.operations[{idx}]");
        if !ids.insert(operation.id.as_str()) {
            push_error(
                output,
                "operation.id.duplicate",
                format!("duplicate operation id `{}`", operation.id),
                Some(format!("{operation_path}.id")),
            );
        }
        if let Err(error) = operation.validate() {
            push_error(
                output,
                "operation.invalid",
                error.to_string(),
                Some(operation_path.clone()),
            );
        }
        if let PluginOperationTarget::Tool { tool } = &operation.target
            && !known_tools.contains(tool.as_str())
        {
            push_error(
                output,
                "operation.target.tool.unknown",
                format!("operation references unknown local tool `{tool}`"),
                Some(format!("{operation_path}.target.tool")),
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::validate_plugin_target;

    static VALIDATION_FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(1);

    fn validate_config(value: serde_json::Value) -> super::PluginValidateOutput {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        let counter = VALIDATION_FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!(
            "/tmp/agena-plugin-validation-{}-{nonce}-{counter}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&value).expect("encode validation fixture"),
        )
        .expect("write validation fixture");
        let output = validate_plugin_target(&path, false).expect("validate fixture");
        let _ = std::fs::remove_file(path);
        output
    }

    #[test]
    fn plugin_validation_reports_missing_required_dependencies() {
        let output = validate_config(serde_json::json!({
            "plugins": {
                "list": {
                    "example.consumer": {
                        "package": { "kind": "static" },
                        "activation": { "requires": ["example.provider"] }
                    }
                }
            }
        }));

        assert!(!output.ok);
        assert!(output.errors.iter().any(|message| {
            message.code == "config.plugin.activation.required_dependency_unavailable"
                && message.message.contains("example.provider")
        }));
    }

    #[test]
    fn plugin_validation_accepts_missing_soft_ordering_hints() {
        let output = validate_config(serde_json::json!({
            "plugins": {
                "list": {
                    "example.consumer": {
                        "package": { "kind": "static" },
                        "activation": { "after": ["example.optional-observer"] }
                    }
                }
            }
        }));

        assert!(output.ok, "soft ordering hints must not block: {output:#?}");
    }

    #[test]
    fn plugin_validation_reports_required_dependency_cycles() {
        let output = validate_config(serde_json::json!({
            "plugins": {
                "list": {
                    "example.a": {
                        "package": { "kind": "static" },
                        "activation": { "requires": ["example.b"] }
                    },
                    "example.b": {
                        "package": { "kind": "static" },
                        "activation": { "requires": ["example.a"] }
                    }
                }
            }
        }));

        assert_eq!(
            output
                .errors
                .iter()
                .filter(|message| {
                    message.code == "config.plugin.activation.required_dependency_cycle"
                })
                .count(),
            2
        );
    }

    #[test]
    fn plugin_validation_applies_inherited_profiles_before_activation() {
        let output = validate_config(serde_json::json!({
            "plugins": {
                "list": {
                    "example.consumer": {
                        "package": { "kind": "static" },
                        "activation": { "requires": ["example.provider"] },
                        "settings": { "mode": "base" }
                    },
                    "example.provider": {
                        "enabled": false,
                        "package": { "kind": "static" }
                    }
                },
                "profiles": {
                    "base": {
                        "plugins": {
                            "example.provider": {
                                "action": "patch",
                                "enabled": true
                            }
                        }
                    },
                    "coding": {
                        "extends": ["base"],
                        "plugins": {
                            "example.consumer": {
                                "action": "patch",
                                "settings_patch": { "mode": "coding" }
                            }
                        }
                    }
                },
                "active_profiles": ["coding"]
            }
        }));

        assert!(
            output.ok,
            "profile must enable provider before activation: {output:#?}"
        );
    }

    #[test]
    fn plugin_validation_reports_invalid_profile_patch_targets() {
        let output = validate_config(serde_json::json!({
            "plugins": {
                "profiles": {
                    "workspace": {
                        "plugins": {
                            "example.missing": {
                                "action": "patch",
                                "enabled": false
                            }
                        }
                    }
                },
                "active_profiles": ["workspace"]
            }
        }));

        assert!(!output.ok);
        assert!(output.errors.iter().any(|message| {
            message.code == "config.plugin.profile.invalid"
                && message.message.contains("example.missing")
                && message.message.contains("action=replace")
        }));
    }

    #[test]
    fn plugin_validation_reports_profile_inheritance_cycles() {
        let output = validate_config(serde_json::json!({
            "plugins": {
                "profiles": {
                    "a": { "extends": ["b"] },
                    "b": { "extends": ["a"] }
                },
                "active_profiles": ["a"]
            }
        }));

        assert!(!output.ok);
        assert!(output.errors.iter().any(|message| {
            message.code == "config.plugin.profile.invalid" && message.message.contains("cycle")
        }));
    }
}
