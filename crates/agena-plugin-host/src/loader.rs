//! Loader for one configured plugin.

use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use crate::config::{ConfiguredPlugin, PluginPackage, PluginSignature};
use crate::error::{HostError, TransportError};
use crate::host::{HostHandle, LoadedPlugin};
use crate::registry::{
    effective_capabilities_for_manifest, per_tool_capabilities, validate_tool_definition,
};
use crate::sdk::{InitContext, InitOutcome, PluginKey, PluginManifest};
use crate::sdk::{rpc::method, schema_validation};
use crate::transport::{
    PluginTransport, cdylib::CdylibTransport, http::HttpTransport, stdio::StdioTransport,
};

pub struct StaticRegistration {
    pub builder: Box<dyn FnOnce() -> Arc<dyn PluginTransport> + Send + Sync>,
}

#[allow(clippy::too_many_arguments)]
pub async fn load_entry(
    plugin_id: &str,
    configured_plugin: &ConfiguredPlugin,
    static_registry: &mut std::collections::HashMap<PluginKey, StaticRegistration>,
    host_handle: Arc<HostHandle>,
    agena_version: &str,
    workspace_root: &Path,
    env_lookup: &(dyn Fn(&str) -> Option<String> + Send + Sync),
    trusted_keys: &std::collections::BTreeMap<String, String>,
) -> Result<LoadedPlugin, HostError> {
    let transport: Arc<dyn PluginTransport> = match &configured_plugin.package {
        PluginPackage::Static { .. } => {
            let plugin_key: PluginKey = plugin_id.parse().map_err(|err| HostError::Load {
                plugin: plugin_id.to_string(),
                message: format!("invalid plugin id `{plugin_id}`: {err}"),
            })?;
            let registration =
                static_registry
                    .remove(&plugin_key)
                    .ok_or_else(|| HostError::Load {
                        plugin: plugin_id.to_string(),
                        message: format!("no static plugin registered with id `{plugin_id}`"),
                    })?;
            (registration.builder)()
        }
        PluginPackage::Cdylib {
            path,
            sha256,
            signature,
            ..
        } => {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                workspace_root.join(path)
            };
            #[cfg(feature = "signing")]
            {
                if let Some(expected) = sha256 {
                    verify_sha256(&resolved, expected).map_err(|e| HostError::Load {
                        plugin: plugin_id.to_string(),
                        message: e,
                    })?;
                }
                if let Some(sig) = signature {
                    verify_signature(&resolved, sig, trusted_keys).map_err(|e| {
                        HostError::Load {
                            plugin: plugin_id.to_string(),
                            message: e,
                        }
                    })?;
                }
            }
            #[cfg(not(feature = "signing"))]
            {
                if sha256.is_some() || signature.is_some() {
                    return Err(HostError::Load {
                        plugin: plugin_id.to_string(),
                        message:
                            "plugin signing fields are set but the `signing` feature is disabled"
                                .into(),
                    });
                }
                let _ = (sha256, signature, trusted_keys);
            }
            let t = CdylibTransport::load(&resolved).map_err(|e| HostError::Load {
                plugin: plugin_id.to_string(),
                message: e.to_string(),
            })?;
            Arc::new(t)
        }
        PluginPackage::Stdio {
            command,
            args,
            env,
            cwd,
            restart,
            sha256,
            ..
        } => {
            #[cfg(feature = "signing")]
            {
                if let Some(expected) = sha256 {
                    let cmd_path = std::path::Path::new(command);
                    if cmd_path.exists() {
                        verify_sha256(cmd_path, expected).map_err(|e| HostError::Load {
                            plugin: plugin_id.to_string(),
                            message: e,
                        })?;
                    }
                }
            }
            #[cfg(not(feature = "signing"))]
            {
                if sha256.is_some() {
                    return Err(HostError::Load {
                        plugin: plugin_id.to_string(),
                        message: "stdio.sha256 set but the `signing` feature is disabled".into(),
                    });
                }
                let _ = sha256;
            }
            let env_map: std::collections::HashMap<String, String> =
                env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let host_handler = host_handle.host_handler_for(plugin_id.to_string());
            let status_sink = host_handle.status_registry();
            let log_sink = host_handle.log_store();
            let plugin_key: PluginKey = plugin_id.parse().map_err(|err| HostError::Load {
                plugin: plugin_id.to_string(),
                message: format!("invalid plugin id `{plugin_id}`: {err}"),
            })?;
            let t = StdioTransport::spawn_with_policy_and_status(
                command,
                args,
                &env_map,
                cwd.as_ref(),
                Some(host_handler),
                restart.clone(),
                Some(plugin_key),
                Some(status_sink),
                Some(log_sink),
            )
            .await
            .map_err(|e| HostError::Load {
                plugin: plugin_id.to_string(),
                message: e.to_string(),
            })?;
            Arc::new(t)
        }
        PluginPackage::Http { url, auth, .. } => {
            let t = HttpTransport::new(
                url.clone(),
                auth.clone(),
                env_lookup,
                host_handle.callback_url(plugin_id).is_some(),
            );
            Arc::new(t)
        }
        #[cfg(feature = "wasm")]
        PluginPackage::Wasm { path, sha256, .. } => {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                workspace_root.join(path)
            };
            // Optional supply-chain check before loading.
            if let Some(expected) = sha256 {
                verify_sha256(&resolved, expected).map_err(|e| HostError::Load {
                    plugin: plugin_id.to_string(),
                    message: e,
                })?;
            }
            let t = crate::transport::wasm::WasmTransport::load(&resolved).map_err(|e| {
                HostError::Load {
                    plugin: plugin_id.to_string(),
                    message: e.to_string(),
                }
            })?;
            Arc::new(t)
        }
        #[cfg(not(feature = "wasm"))]
        PluginPackage::Wasm { .. } => {
            return Err(HostError::Load {
                plugin: plugin_id.to_string(),
                message: "wasm transport requires the `wasm` feature".into(),
            });
        }
    };

    let initialization = initialize_transport(
        plugin_id,
        configured_plugin,
        &host_handle,
        agena_version,
        workspace_root,
        trusted_keys,
        Arc::clone(&transport),
    )
    .await;

    if initialization.is_err() {
        // A stdio transport owns a child process and does not kill it merely
        // because the final Arc is dropped. Failed initialization must close
        // every transport explicitly before the host proceeds.
        let _ = transport.close().await;
    }
    initialization
}

#[allow(clippy::too_many_arguments)]
async fn initialize_transport(
    plugin_id: &str,
    configured_plugin: &ConfiguredPlugin,
    host_handle: &Arc<HostHandle>,
    agena_version: &str,
    workspace_root: &Path,
    trusted_keys: &std::collections::BTreeMap<String, String>,
    transport: Arc<dyn PluginTransport>,
) -> Result<LoadedPlugin, HostError> {
    transport
        .attach_host(host_handle.scoped_host_client(plugin_id.to_string()))
        .await
        .map_err(|e| HostError::Load {
            plugin: plugin_id.to_string(),
            message: e.to_string(),
        })?;

    let prefetched_manifest_value = transport
        .dispatch(
            method::META_MANIFEST,
            serde_json::Value::Object(Default::default()),
        )
        .await
        .map_err(|e| HostError::Init {
            plugin: plugin_id.to_string(),
            message: format!("{e}"),
        })?;
    let prefetched_manifest: PluginManifest = serde_json::from_value(prefetched_manifest_value)
        .map_err(|e| HostError::Init {
            plugin: plugin_id.to_string(),
            message: e.to_string(),
        })?;
    let plugin_key: PluginKey = plugin_id.parse().map_err(|err| HostError::Init {
        plugin: plugin_id.to_string(),
        message: format!("invalid plugin id `{plugin_id}`: {err}"),
    })?;
    validate_manifest(
        plugin_id,
        &plugin_key,
        &prefetched_manifest,
        "meta/manifest",
    )?;
    validate_manifest_config(plugin_id, &prefetched_manifest, configured_plugin.config())?;
    host_handle.set_plugin_manifest_name(plugin_key.clone(), prefetched_manifest.name.clone());
    host_handle
        .set_plugin_capabilities(
            plugin_key.clone(),
            effective_capabilities_for_manifest(
                &prefetched_manifest.tools,
                &prefetched_manifest.plugin_capabilities,
            ),
        )
        .await;
    host_handle
        .set_plugin_tool_capabilities(
            plugin_key.clone(),
            per_tool_capabilities(&prefetched_manifest.tools),
        )
        .await;

    let init_ctx = InitContext {
        agena_version: agena_version.to_string(),
        workspace_root: workspace_root.to_path_buf(),
        plugin_id: plugin_key.clone(),
        host_callback_url: host_handle.callback_url(plugin_id),
        host_callback_token: host_handle.callback_token(plugin_id).await,
        config: configured_plugin.config().clone(),
        protocol_version: crate::sdk::rpc::PROTOCOL_VERSION,
    };

    let init_params = serde_json::to_value(&init_ctx).map_err(|e| HostError::Init {
        plugin: plugin_id.to_string(),
        message: e.to_string(),
    })?;

    let outcome_value = transport
        .dispatch(method::META_INIT, init_params)
        .await
        .map_err(|e| HostError::Init {
            plugin: plugin_id.to_string(),
            message: format!("{e}"),
        })?;

    let outcome: InitOutcome =
        serde_json::from_value(outcome_value).map_err(|e| HostError::Init {
            plugin: plugin_id.to_string(),
            message: e.to_string(),
        })?;

    validate_manifest(plugin_id, &plugin_key, &outcome.manifest, "meta/init")?;
    if outcome.manifest != prefetched_manifest {
        return Err(HostError::Init {
            plugin: plugin_id.to_string(),
            message: "plugin manifest changed between `meta/manifest` and `meta/init`; manifests must be immutable during initialization".to_string(),
        });
    }
    let trust_level = plugin_trust_level(configured_plugin, trusted_keys);
    let provenance = plugin_provenance(configured_plugin, trusted_keys);

    Ok(LoadedPlugin::new(
        configured_plugin.kind_str(),
        configured_plugin.clone(),
        transport,
        outcome.manifest,
        trust_level,
        provenance,
    ))
}

fn plugin_trust_level(
    configured_plugin: &ConfiguredPlugin,
    trusted_keys: &std::collections::BTreeMap<String, String>,
) -> String {
    match &configured_plugin.package {
        PluginPackage::Static { .. } => "static".to_string(),
        PluginPackage::Cdylib {
            signature, sha256, ..
        } => {
            if has_trusted_signature(signature.as_ref(), trusted_keys) {
                "verified".to_string()
            } else if sha256.is_some() {
                "checksummed".to_string()
            } else {
                "unverified".to_string()
            }
        }
        PluginPackage::Stdio { sha256, .. } => {
            if sha256.is_some() {
                "checksummed".to_string()
            } else {
                "unverified".to_string()
            }
        }
        PluginPackage::Http { .. } => "remote".to_string(),
        PluginPackage::Wasm { sha256, .. } => {
            if sha256.is_some() {
                "checksummed".to_string()
            } else {
                "unverified".to_string()
            }
        }
    }
}

fn plugin_provenance(
    configured_plugin: &ConfiguredPlugin,
    trusted_keys: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let mut provenance = vec![format!("transport:{}", configured_plugin.kind_str())];
    match &configured_plugin.package {
        PluginPackage::Static { .. } => provenance.push("static registration".to_string()),
        PluginPackage::Cdylib {
            path,
            sha256,
            signature,
            ..
        } => {
            provenance.push(format!("path:{}", path.display()));
            if sha256.is_some() {
                provenance.push("sha256 configured".to_string());
            }
            if let Some(signature) = signature {
                provenance.push(format!("signature key:{}", signature.key_id));
                if trusted_keys.contains_key(&signature.key_id) {
                    provenance.push("signature key trusted".to_string());
                }
            }
        }
        PluginPackage::Stdio {
            command,
            sha256,
            cwd,
            ..
        } => {
            provenance.push(format!("command:{}", command));
            if let Some(cwd) = cwd {
                provenance.push(format!("cwd:{}", cwd.display()));
            }
            if sha256.is_some() {
                provenance.push("sha256 configured".to_string());
            }
        }
        PluginPackage::Http { url, .. } => {
            provenance.push(format!("url:{}", url));
        }
        PluginPackage::Wasm { path, sha256, .. } => {
            provenance.push(format!("path:{}", path.display()));
            if sha256.is_some() {
                provenance.push("sha256 configured".to_string());
            }
        }
    }
    provenance
}

fn has_trusted_signature(
    signature: Option<&PluginSignature>,
    trusted_keys: &std::collections::BTreeMap<String, String>,
) -> bool {
    signature.is_some_and(|signature| trusted_keys.contains_key(&signature.key_id))
}

const SUPPORTED_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[allow(clippy::result_large_err)]
fn validate_manifest(
    plugin_id: &str,
    configured_key: &PluginKey,
    manifest: &PluginManifest,
    phase: &str,
) -> Result<(), HostError> {
    let fail = |message: String| HostError::Init {
        plugin: plugin_id.to_string(),
        message: format!("invalid manifest returned by `{phase}`: {message}"),
    };

    if manifest.schema_version != SUPPORTED_MANIFEST_SCHEMA_VERSION {
        return Err(fail(format!(
            "unsupported schema version {}; expected {}",
            manifest.schema_version, SUPPORTED_MANIFEST_SCHEMA_VERSION
        )));
    }

    if manifest.namespace.trim() != manifest.namespace || manifest.name.trim() != manifest.name {
        return Err(fail(
            "plugin namespace and name must not contain leading or trailing whitespace".to_string(),
        ));
    }
    let manifest_key = PluginKey::new(manifest.namespace.clone(), manifest.name.clone())
        .map_err(|error| fail(format!("invalid plugin identity: {error}")))?;
    if &manifest_key != configured_key {
        return Err(fail(format!(
            "plugin identity `{manifest_key}` does not match configured id `{configured_key}`"
        )));
    }
    if manifest.version.trim().is_empty() || manifest.version.trim() != manifest.version {
        return Err(fail(
            "plugin version must be non-empty and must not contain leading or trailing whitespace"
                .to_string(),
        ));
    }

    let mut tool_names = BTreeSet::new();
    for definition in &manifest.tools {
        validate_tool_definition(configured_key, definition).map_err(&fail)?;
        if !tool_names.insert(definition.name.as_str()) {
            return Err(fail(format!("duplicate tool name `{}`", definition.name)));
        }
    }

    let validate_id = |id: &str,
                       label: &str,
                       seen: &mut BTreeSet<String>|
     -> Result<(), HostError> {
        if id.trim().is_empty() || id.trim() != id {
            return Err(fail(format!(
                "{label} id `{id}` must be non-empty and must not contain leading or trailing whitespace"
            )));
        }
        if !seen.insert(id.to_owned()) {
            return Err(fail(format!("duplicate {label} id `{id}`")));
        }
        Ok(())
    };

    let mut command_ids = BTreeSet::new();
    let mut action_ids = BTreeSet::new();
    for command in &manifest.commands {
        validate_id(command.id.as_str(), "command", &mut command_ids)?;
        validate_id(command.id.as_str(), "studio action", &mut action_ids)?;
    }

    let mut statusline_ids = BTreeSet::new();
    for segment in &manifest.ui.tui.statusline_segments {
        validate_id(
            segment.id.as_str(),
            "statusline segment",
            &mut statusline_ids,
        )?;
    }
    let mut theme_ids = BTreeSet::new();
    for theme in &manifest.ui.tui.themes {
        validate_id(theme.id.as_str(), "theme", &mut theme_ids)?;
    }
    let mut content_block_ids = BTreeSet::new();
    for block in &manifest.ui.tui.content_blocks {
        validate_id(
            block.id.as_str(),
            "TUI content block",
            &mut content_block_ids,
        )?;
    }
    for control in &manifest.ui.studio.controls {
        validate_id(control.id.as_str(), "studio action", &mut action_ids)?;
    }
    let mut view_ids = BTreeSet::new();
    for view in &manifest.ui.studio.views {
        validate_id(view.id.as_str(), "studio view", &mut view_ids)?;
        for control in &view.controls {
            validate_id(control.id.as_str(), "studio action", &mut action_ids)?;
        }
    }

    Ok(())
}

#[allow(clippy::result_large_err)]
fn validate_manifest_config(
    plugin_id: &str,
    manifest: &PluginManifest,
    config: &JsonValue,
) -> Result<(), HostError> {
    if config.is_null() {
        return Ok(());
    }
    let Some(schema) = manifest.config_schema.as_ref() else {
        return Ok(());
    };
    validate_json_schema_value(schema, config).map_err(|message| {
        HostError::Config(format!(
            "plugin `{plugin_id}` config does not match manifest schema: {message}"
        ))
    })
}

/// Validate a JSON value against the schema subset supported by Agena plugin
/// manifests and generated tool contracts.
///
/// This is intentionally the same validator used for plugin configuration so
/// Tool API callers can reject definitely invalid execution-tool input before
/// the tool handler runs. Agena extension aliases (`x-agena-aliases`) are
/// accepted wherever their canonical property is accepted.
pub fn validate_json_schema_value(schema: &JsonValue, value: &JsonValue) -> Result<(), String> {
    validate_schema_value("$", schema, schema, value)
}

fn validate_schema_value(
    path: &str,
    root: &JsonValue,
    schema: &JsonValue,
    value: &JsonValue,
) -> Result<(), String> {
    let schema = resolve_schema(root, schema);
    match schema {
        JsonValue::Bool(true) => return Ok(()),
        JsonValue::Bool(false) => {
            return Err(format!("{path}: schema rejects this value"));
        }
        JsonValue::Object(_) => {}
        _ => return Ok(()),
    }

    let schema_obj = schema.as_object().expect("object already checked");

    if let Some(all_of) = schema_obj.get("allOf").and_then(JsonValue::as_array) {
        for branch in all_of {
            validate_schema_value(path, root, branch, value)?;
        }
    }

    if let Some(any_of) = schema_obj.get("anyOf").and_then(JsonValue::as_array) {
        let matching = any_of
            .iter()
            .find(|branch| schema_matches(root, branch, value))
            .ok_or_else(|| format!("{path}: value must match at least one allowed shape"))?;
        validate_schema_value(path, root, matching, value)?;
    }

    if let Some(one_of) = schema_obj.get("oneOf").and_then(JsonValue::as_array) {
        let matching = one_of
            .iter()
            .filter(|branch| schema_matches(root, branch, value))
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(format!(
                "{path}: value must match exactly one allowed shape"
            ));
        }
        validate_schema_value(path, root, matching[0], value)?;
    }

    if let Some(expected) = schema_obj.get("const")
        && expected != value
    {
        return Err(format!("{path}: value must equal {expected}"));
    }

    if let Some(variants) = schema_obj.get("enum").and_then(|v| v.as_array())
        && !variants.iter().any(|candidate| candidate == value)
    {
        return Err(format!(
            "{path}: value is not one of the allowed enum variants"
        ));
    }

    if let Some(expected_type) = schema_obj.get("type") {
        validate_schema_type(path, expected_type, value)?;
    }

    if let Some(if_schema) = schema_obj.get("if") {
        let target = if schema_matches(root, if_schema, value) {
            schema_obj.get("then")
        } else {
            schema_obj.get("else")
        };
        if let Some(target_schema) = target {
            validate_schema_value(path, root, target_schema, value)?;
        }
    }

    if let Some(required) = schema_obj.get("required").and_then(|v| v.as_array()) {
        let object = value
            .as_object()
            .ok_or_else(|| format!("{path}: required fields require an object value"))?;
        for field in required {
            let Some(field) = field.as_str() else {
                continue;
            };
            if !object.contains_key(field)
                && !schema_property_aliases(schema_obj, field)
                    .any(|alias| object.contains_key(alias))
            {
                return Err(format!("{path}: missing required property '{field}'"));
            }
        }
    }

    if let Some(object) = value.as_object() {
        validate_object_schema(path, root, schema, schema_obj, object)?;
    }

    if let Some(items) = value.as_array() {
        validate_array_schema(path, root, schema, schema_obj, items)?;
    }

    if let Some(text) = value.as_str() {
        validate_string_schema(path, schema_obj, text)?;
    }

    if let Some(number) = value.as_f64() {
        validate_number_schema(path, schema_obj, number)?;
    }

    Ok(())
}

fn validate_object_schema(
    path: &str,
    root: &JsonValue,
    schema: &JsonValue,
    schema_object: &JsonMap<String, JsonValue>,
    value: &JsonMap<String, JsonValue>,
) -> Result<(), String> {
    if let Some(patterns) = schema_object
        .get("patternProperties")
        .and_then(JsonValue::as_object)
    {
        for pattern in patterns.keys() {
            schema_validation::validate_regex_pattern(pattern).map_err(|error| {
                format!("{path}: invalid patternProperties regex `{pattern}`: {error}")
            })?;
        }
    }
    if let Some(min_properties) = schema_object
        .get("minProperties")
        .and_then(JsonValue::as_u64)
        && value.len() < min_properties as usize
    {
        return Err(format!(
            "{path}: object must contain at least {min_properties} field(s)"
        ));
    }
    if let Some(max_properties) = schema_object
        .get("maxProperties")
        .and_then(JsonValue::as_u64)
        && value.len() > max_properties as usize
    {
        return Err(format!(
            "{path}: object must contain at most {max_properties} field(s)"
        ));
    }
    if let Some(property_names_schema) = schema_object.get("propertyNames") {
        for key in value.keys() {
            validate_schema_value(
                format!("{path}.{key}").as_str(),
                root,
                property_names_schema,
                &JsonValue::String(key.clone()),
            )?;
        }
    }
    for (key, child_value) in value {
        let child_path = format!("{path}.{key}");
        if let Some(child_schema) = object_property_schema(root, schema, key) {
            validate_schema_value(&child_path, root, &child_schema, child_value)?;
        } else if schema_object.get("additionalProperties") == Some(&JsonValue::Bool(false)) {
            return Err(format!("{path}: unexpected property '{key}'"));
        } else if let Some(additional) = schema_object.get("additionalProperties")
            && !matches!(additional, JsonValue::Bool(true))
        {
            validate_schema_value(&child_path, root, additional, child_value)?;
        }
    }
    if let Some(dependencies) = schema_object
        .get("dependentRequired")
        .and_then(JsonValue::as_object)
    {
        for (trigger, required_fields) in dependencies {
            if !value.contains_key(trigger) {
                continue;
            }
            for required in required_fields
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
            {
                if !value.contains_key(required) {
                    return Err(format!(
                        "{path}: missing required property '{required}' because '{trigger}' is set"
                    ));
                }
            }
        }
    }
    if let Some(dependencies) = schema_object
        .get("dependentSchemas")
        .and_then(JsonValue::as_object)
    {
        for (trigger, dependency_schema) in dependencies {
            if value.contains_key(trigger) {
                validate_schema_value(
                    path,
                    root,
                    dependency_schema,
                    &JsonValue::Object(value.clone()),
                )?;
            }
        }
    }
    Ok(())
}

fn validate_array_schema(
    path: &str,
    root: &JsonValue,
    schema: &JsonValue,
    schema_object: &JsonMap<String, JsonValue>,
    value: &[JsonValue],
) -> Result<(), String> {
    if let Some(min_items) = schema_object.get("minItems").and_then(JsonValue::as_u64)
        && value.len() < min_items as usize
    {
        return Err(format!(
            "{path}: array must contain at least {min_items} item(s)"
        ));
    }
    if let Some(max_items) = schema_object.get("maxItems").and_then(JsonValue::as_u64)
        && value.len() > max_items as usize
    {
        return Err(format!(
            "{path}: array must contain at most {max_items} item(s)"
        ));
    }
    if schema_object
        .get("uniqueItems")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let mut seen = std::collections::BTreeSet::new();
        for item in value {
            if !seen.insert(item.to_string()) {
                return Err(format!("{path}: array contains duplicate items"));
            }
        }
    }
    if let Some(contains_schema) = schema_object.get("contains") {
        let matches = value
            .iter()
            .filter(|item| schema_matches(root, contains_schema, item))
            .count();
        let min_contains = schema_object
            .get("minContains")
            .and_then(JsonValue::as_u64)
            .unwrap_or(1);
        let max_contains = schema_object.get("maxContains").and_then(JsonValue::as_u64);
        if matches < min_contains as usize {
            return Err(format!(
                "{path}: array must contain at least {min_contains} matching item(s)"
            ));
        }
        if let Some(max_contains) = max_contains
            && matches > max_contains as usize
        {
            return Err(format!(
                "{path}: array must contain at most {max_contains} matching item(s)"
            ));
        }
    }
    for (index, item) in value.iter().enumerate() {
        if let Some(item_schema) = array_item_schema(root, schema, index) {
            validate_schema_value(
                format!("{path}[{index}]").as_str(),
                root,
                &item_schema,
                item,
            )?;
        }
    }
    Ok(())
}

fn validate_string_schema(
    path: &str,
    schema_object: &JsonMap<String, JsonValue>,
    text: &str,
) -> Result<(), String> {
    if let Some(min_length) = schema_object.get("minLength").and_then(JsonValue::as_u64)
        && text.chars().count() < min_length as usize
    {
        return Err(format!(
            "{path}: string is shorter than minLength {min_length}"
        ));
    }
    if let Some(max_length) = schema_object.get("maxLength").and_then(JsonValue::as_u64)
        && text.chars().count() > max_length as usize
    {
        return Err(format!(
            "{path}: string is longer than maxLength {max_length}"
        ));
    }
    if let Some(format) = schema_object.get("format").and_then(JsonValue::as_str)
        && !schema_validation::format_is_valid(format, text)
    {
        return Err(format!("{path}: string must match format {format}"));
    }
    if let Some(pattern) = schema_object.get("pattern").and_then(JsonValue::as_str) {
        match schema_validation::pattern_matches(pattern, text) {
            Ok(true) => {}
            Ok(false) => return Err(format!("{path}: string must match pattern {pattern}")),
            Err(error) => {
                return Err(format!(
                    "{path}: invalid regex pattern `{pattern}`: {error}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_number_schema(
    path: &str,
    schema_object: &JsonMap<String, JsonValue>,
    number: f64,
) -> Result<(), String> {
    if let Some(minimum) = schema_object.get("minimum").and_then(JsonValue::as_f64)
        && number < minimum
    {
        return Err(format!("{path}: value must be >= {minimum}"));
    }
    if let Some(maximum) = schema_object.get("maximum").and_then(JsonValue::as_f64)
        && number > maximum
    {
        return Err(format!("{path}: value must be <= {maximum}"));
    }
    if let Some(minimum) = schema_object
        .get("exclusiveMinimum")
        .and_then(JsonValue::as_f64)
        && number <= minimum
    {
        return Err(format!("{path}: value must be > {minimum}"));
    }
    if let Some(maximum) = schema_object
        .get("exclusiveMaximum")
        .and_then(JsonValue::as_f64)
        && number >= maximum
    {
        return Err(format!("{path}: value must be < {maximum}"));
    }
    if let Some(multiple_of) = schema_object.get("multipleOf").and_then(JsonValue::as_f64)
        && multiple_of > 0.0
    {
        let quotient = number / multiple_of;
        if (quotient - quotient.round()).abs() > f64::EPSILON {
            return Err(format!("{path}: value must be a multiple of {multiple_of}"));
        }
    }
    Ok(())
}

fn schema_matches(root: &JsonValue, schema: &JsonValue, value: &JsonValue) -> bool {
    validate_schema_value("$match", root, schema, value).is_ok()
}

fn resolve_schema<'a>(root: &'a JsonValue, schema: &'a JsonValue) -> &'a JsonValue {
    let Some(reference) = schema.get("$ref").and_then(JsonValue::as_str) else {
        return schema;
    };
    if !reference.starts_with("#/") {
        return schema;
    }
    let mut cursor = root;
    for segment in reference.trim_start_matches("#/").split('/') {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        let Some(next) = cursor.get(segment.as_str()) else {
            return schema;
        };
        cursor = next;
    }
    cursor
}

fn combine_schema_constraints(mut schemas: Vec<JsonValue>) -> Option<JsonValue> {
    match schemas.len() {
        0 => None,
        1 => schemas.pop(),
        _ => {
            let mut object = JsonMap::new();
            object.insert("allOf".to_owned(), JsonValue::Array(schemas));
            Some(JsonValue::Object(object))
        }
    }
}

fn object_property_schema(root: &JsonValue, schema: &JsonValue, key: &str) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    let mut matches = Vec::new();
    let mut matched_named_or_pattern = false;

    if let Some(properties) = schema.get("properties").and_then(JsonValue::as_object) {
        if let Some(child) = properties.get(key) {
            matches.push(child.clone());
            matched_named_or_pattern = true;
        } else if let Some(child) = properties.values().find(|child| {
            child
                .get("x-agena-aliases")
                .and_then(JsonValue::as_array)
                .is_some_and(|aliases| aliases.iter().any(|alias| alias.as_str() == Some(key)))
        }) {
            matches.push(child.clone());
            matched_named_or_pattern = true;
        }
    }
    if let Some(patterns) = schema
        .get("patternProperties")
        .and_then(JsonValue::as_object)
    {
        for (pattern, child) in patterns {
            if pattern_key_matches(pattern, key) {
                matches.push(child.clone());
                matched_named_or_pattern = true;
            }
        }
    }
    if !matched_named_or_pattern {
        match schema.get("additionalProperties") {
            Some(JsonValue::Object(object)) => matches.push(JsonValue::Object(object.clone())),
            Some(other) if !matches!(other, JsonValue::Bool(true) | JsonValue::Bool(false)) => {
                matches.push(other.clone());
            }
            _ => {}
        }
    }
    combine_schema_constraints(matches)
}

fn schema_property_aliases<'a>(
    schema_object: &'a JsonMap<String, JsonValue>,
    property: &str,
) -> impl Iterator<Item = &'a str> {
    schema_object
        .get("properties")
        .and_then(JsonValue::as_object)
        .and_then(|properties| properties.get(property))
        .and_then(|property_schema| property_schema.get("x-agena-aliases"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
}

fn array_item_schema(root: &JsonValue, schema: &JsonValue, index: usize) -> Option<JsonValue> {
    let schema = resolve_schema(root, schema);
    if let Some(prefix) = schema.get("prefixItems").and_then(JsonValue::as_array)
        && let Some(item) = prefix.get(index)
    {
        return Some(item.clone());
    }
    schema.get("items").cloned()
}

fn validate_schema_type(path: &str, expected: &JsonValue, value: &JsonValue) -> Result<(), String> {
    let matches = match expected {
        JsonValue::String(kind) => value_matches_type(kind, value),
        JsonValue::Array(kinds) => kinds
            .iter()
            .filter_map(|kind| kind.as_str())
            .any(|kind| value_matches_type(kind, value)),
        _ => true,
    };
    if matches {
        Ok(())
    } else {
        Err(format!("{path}: value does not match declared schema type"))
    }
}

fn value_matches_type(kind: &str, value: &JsonValue) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        _ => true,
    }
}

fn pattern_key_matches(pattern: &str, key: &str) -> bool {
    schema_validation::pattern_matches(pattern, key).unwrap_or(false)
}

pub async fn shutdown_transport(transport: Arc<dyn PluginTransport>) -> Result<(), TransportError> {
    let _ = transport
        .dispatch(
            method::META_SHUTDOWN,
            serde_json::Value::Object(Default::default()),
        )
        .await;
    transport.close().await
}

/// Verify the sha256 of a file against an expected hex digest. Used by both
/// the wasm transport (for safety) and the signing helpers.
#[cfg(any(feature = "wasm", feature = "signing"))]
pub fn verify_sha256(path: &std::path::Path, expected_hex: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).map_err(|e| format!("read `{}`: {e}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    let got = hex::encode(digest);
    if got.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(format!(
            "sha256 mismatch for `{}`: expected {}, got {}",
            path.display(),
            expected_hex,
            got
        ))
    }
}

/// Verify an ed25519 signature over the file bytes against a trusted key.
#[cfg(feature = "signing")]
pub fn verify_signature(
    path: &std::path::Path,
    sig: &crate::config::PluginSignature,
    trusted_keys: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read `{}`: {e}", path.display()))?;
    verify_signature_bytes(&bytes, sig, trusted_keys).map_err(|e| {
        format!(
            "signature verification failed for `{}`: {e}",
            path.display()
        )
    })
}

/// Verify an ed25519 signature against in-memory bytes. Used by the marketplace
/// after a download but before the artifact lands on disk.
#[cfg(feature = "signing")]
pub fn verify_signature_bytes(
    bytes: &[u8],
    sig: &crate::config::PluginSignature,
    trusted_keys: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let key_hex = trusted_keys
        .get(&sig.key_id)
        .ok_or_else(|| format!("unknown trusted key id `{}`", sig.key_id))?;
    let key_bytes = hex::decode(key_hex)
        .map_err(|e| format!("trusted key `{}` is not valid hex: {e}", sig.key_id))?;
    let key_array: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| format!("trusted key `{}` must be 32 bytes", sig.key_id))?;
    let verifier = VerifyingKey::from_bytes(&key_array)
        .map_err(|e| format!("invalid ed25519 public key `{}`: {e}", sig.key_id))?;
    let sig_bytes =
        hex::decode(&sig.signature).map_err(|e| format!("signature is not valid hex: {e}"))?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| "signature must be 64 bytes".to_string())?;
    let signature = Signature::from_bytes(&sig_array);
    verifier
        .verify(bytes, &signature)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod manifest_tests {
    use super::{validate_json_schema_value, validate_manifest};
    use crate::sdk::{
        PluginCommandDefinition, PluginKey, PluginManifest, PluginStudioControl, ToolDefinition,
    };

    fn manifest_with_tools(names: &[&str]) -> PluginManifest {
        let mut manifest = PluginManifest::new("example", "plugin", "1.0.0");
        manifest.tools = names
            .iter()
            .map(|name| {
                serde_json::from_value::<ToolDefinition>(serde_json::json!({ "name": name }))
                    .expect("minimal tool definition")
            })
            .collect();
        manifest
    }

    fn validation_error(manifest: &PluginManifest) -> String {
        validate_manifest(
            "example.plugin",
            &PluginKey::new("example", "plugin").expect("plugin key"),
            manifest,
            "test",
        )
        .expect_err("manifest should be rejected")
        .to_string()
    }

    #[test]
    fn manifest_validation_accepts_dotted_payload_tool_names() {
        let manifest = manifest_with_tools(&["session.rename", "files.read"]);
        validate_manifest(
            "example.plugin",
            &PluginKey::new("example", "plugin").expect("plugin key"),
            &manifest,
            "test",
        )
        .expect("valid manifest");
    }

    #[test]
    fn manifest_validation_rejects_wrong_identity_and_schema_version() {
        let mut manifest = manifest_with_tools(&[]);
        manifest.name = "other".to_string();
        assert!(validation_error(&manifest).contains("does not match configured id"));

        let mut manifest = manifest_with_tools(&[]);
        manifest.schema_version = 2;
        assert!(validation_error(&manifest).contains("unsupported schema version"));
    }

    #[test]
    fn manifest_validation_rejects_blank_whitespace_and_duplicate_tool_names() {
        assert!(validation_error(&manifest_with_tools(&[""])).contains("invalid tool name"));
        assert!(
            validation_error(&manifest_with_tools(&[" session.rename "]))
                .contains("leading or trailing whitespace")
        );
        assert!(
            validation_error(&manifest_with_tools(&["session.rename", "session.rename"]))
                .contains("duplicate tool name")
        );
    }

    #[test]
    fn manifest_validation_accepts_non_object_execution_tool_input_shapes() {
        let mut manifest = manifest_with_tools(&["lookup"]);
        manifest.tools[0].contract.input_schema = serde_json::json!({ "type": "array" });

        validate_manifest(
            "example.plugin",
            &PluginKey::new("example", "plugin").expect("plugin key"),
            &manifest,
            "test",
        )
        .expect("catalog schemas are payload data and may describe any JSON shape");
    }

    #[test]
    fn manifest_validation_rejects_malformed_output_schema_containers() {
        let mut manifest = manifest_with_tools(&["lookup"]);
        manifest.tools[0].contract.output_schema = serde_json::json!("not-a-schema");

        assert!(validation_error(&manifest).contains("output_schema"));
    }

    #[test]
    fn manifest_validation_rejects_duplicate_ui_action_ids() {
        let mut manifest = manifest_with_tools(&[]);
        manifest.commands.push(
            serde_json::from_value::<PluginCommandDefinition>(serde_json::json!({
                "id": "refresh",
                "title": "Refresh"
            }))
            .expect("command definition"),
        );
        manifest.ui.studio.controls.push(
            serde_json::from_value::<PluginStudioControl>(serde_json::json!({
                "id": "refresh",
                "title": "Refresh control"
            }))
            .expect("studio control"),
        );

        assert!(validation_error(&manifest).contains("duplicate studio action id"));
    }

    #[test]
    fn runtime_schema_validation_accepts_declared_property_aliases() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "file_path": {
                    "type": "string",
                    "minLength": 1,
                    "x-agena-aliases": ["path"]
                }
            },
            "required": ["file_path"]
        });

        validate_json_schema_value(&schema, &serde_json::json!({"path": "README.md"}))
            .expect("declared alias should satisfy required and property validation");
        let error = validate_json_schema_value(&schema, &serde_json::json!({"path": ""}))
            .expect_err("an alias must retain the canonical property's constraints");
        assert!(error.contains("minLength 1"));
        let error = validate_json_schema_value(&schema, &serde_json::json!({}))
            .expect_err("missing canonical property and aliases must fail");
        assert!(error.contains("missing required property 'file_path'"));
    }
}
