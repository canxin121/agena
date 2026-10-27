//! Persisted external MCP connections. Live transport operations remain runtime-owned.

use crate::{
    ConfigSettingsEditOptions, ConfigSettingsGetInput, ConfigSettingsLayer, ConfigSettingsSetInput,
    McpServerConfig, read_file_setting, set_layered_file_setting,
};
use agena_domain::mcp_connections::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::PathBuf};

pub const REDACTED: &str = "[REDACTED]";
const RECORD_PATH: &str = "plugins.list.\"agena.mcp\"";
const RECORD_POINTER: &str = "/plugins/list/agena.mcp";
const MAX_IMPORT_CONNECTIONS: usize = 200;
const MAX_IMPORT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpManagementErrorKind {
    Invalid,
    NotFound,
    Conflict,
    Internal,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct McpManagementError {
    pub kind: McpManagementErrorKind,
    pub message: String,
}

impl McpManagementError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: McpManagementErrorKind::Invalid,
            message: message.into(),
        }
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: McpManagementErrorKind::Conflict,
            message: message.into(),
        }
    }
    pub fn not_found() -> Self {
        Self {
            kind: McpManagementErrorKind::NotFound,
            message: "MCP connection was not found in the selected configuration layer.".into(),
        }
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: McpManagementErrorKind::Internal,
            message: message.into(),
        }
    }
}
pub type McpManagementResult<T> = Result<T, McpManagementError>;

#[derive(Clone)]
pub struct McpConnectionStore {
    pub global_path: PathBuf,
    pub workspace_path: PathBuf,
    pub workspace_root: PathBuf,
    active_settings: Option<Value>,
}

struct Layer {
    document: Value,
    revision: String,
}

impl McpConnectionStore {
    pub fn new(global_path: PathBuf, workspace_path: PathBuf, workspace_root: PathBuf) -> Self {
        Self {
            global_path,
            workspace_path,
            workspace_root,
            active_settings: None,
        }
    }

    pub fn with_active_settings(mut self, settings: Value) -> Self {
        self.active_settings = Some(settings);
        self
    }

    pub fn token_store_enabled(&self) -> McpManagementResult<bool> {
        let settings = if let Some(settings) = &self.active_settings {
            settings.clone()
        } else {
            let global = self.load(McpConnectionScope::Global)?;
            let workspace = self.load(McpConnectionScope::Workspace)?;
            merge_mcp_settings(
                record(&global.document)
                    .and_then(|r| r.get("settings"))
                    .unwrap_or(&Value::Null),
                record(&workspace.document)
                    .and_then(|r| r.get("settings"))
                    .unwrap_or(&Value::Null),
            )
        };
        Ok(settings
            .pointer("/runtime/token_store/enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true))
    }

    fn load(&self, scope: McpConnectionScope) -> McpManagementResult<Layer> {
        let path = match scope {
            McpConnectionScope::Global => &self.global_path,
            McpConnectionScope::Workspace => &self.workspace_path,
        };
        let result = read_file_setting(path, ConfigSettingsGetInput::default()).map_err(|_| {
            McpManagementError::internal(
                "Could not read the MCP configuration file. Check its JSON syntax and file access.",
            )
        })?;
        Ok(Layer {
            document: result.value,
            revision: result.revision.unwrap_or_default(),
        })
    }

    fn write_record(
        &self,
        scope: McpConnectionScope,
        layer: &Layer,
        record: Value,
    ) -> McpManagementResult<()> {
        set_layered_file_setting(&self.global_path, &self.workspace_path,
            match scope { McpConnectionScope::Global => ConfigSettingsLayer::Global, McpConnectionScope::Workspace => ConfigSettingsLayer::Workspace },
            ConfigSettingsSetInput { path: RECORD_PATH.into(), value: record,
                options: ConfigSettingsEditOptions { expected_revision: Some(layer.revision.clone()), dry_run: false, validate: true, reload: true } })
            .map_err(|error| {
                if error.to_string().contains("revision changed") {
                    McpManagementError::conflict("Configuration changed while saving. Refresh and retry.")
                } else { McpManagementError::invalid("The updated configuration could not be saved or validated. Check the configuration files and their permissions.") }
            })?;
        Ok(())
    }

    pub fn list(&self) -> McpManagementResult<McpConnectionCatalog> {
        let global = self.load(McpConnectionScope::Global)?;
        let workspace = self.load(McpConnectionScope::Workspace)?;
        self.catalog(&global, &workspace)
    }

    fn catalog(
        &self,
        global: &Layer,
        workspace: &Layer,
    ) -> McpManagementResult<McpConnectionCatalog> {
        let bridge_enabled = record(&workspace.document)
            .or_else(|| record(&global.document))
            .is_none_or(|r| {
                r.get("enabled").and_then(Value::as_bool).unwrap_or(true)
                    && r.pointer("/package/kind")
                        .and_then(Value::as_str)
                        .unwrap_or("static")
                        == "static"
            });
        let mut connections = Vec::new();
        for (scope, layer) in [
            (McpConnectionScope::Global, global),
            (McpConnectionScope::Workspace, workspace),
        ] {
            if let Some(entries) = servers(&layer.document) {
                for (name, config) in entries {
                    let effective = scope == McpConnectionScope::Workspace
                        || !servers(&workspace.document).is_some_and(|s| s.contains_key(name));
                    let mut entry = summary(scope, name, config, effective, bridge_enabled);
                    if entry.state == "disconnected"
                        && let Some(active) = &self.active_settings
                    {
                        let current = active.get("servers").and_then(|s| s.get(name));
                        if current.and_then(|v| validated_value(name, v.clone()).ok())
                            != validated_value(name, config.clone()).ok()
                        {
                            entry.state = "pending_reload".into();
                            entry.error = Some("Saved settings differ from the active runtime. Reload configuration; check profile or command-line overrides if this persists.".into());
                        }
                    }
                    connections.push(entry);
                }
            }
        }
        connections.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(McpConnectionCatalog {
            connections,
            workspace_root: self.workspace_root.to_string_lossy().into_owned(),
            global_path: self.global_path.to_string_lossy().into_owned(),
            workspace_path: self.workspace_path.to_string_lossy().into_owned(),
            bridge_enabled,
        })
    }

    /// Keep redacted presentation, raw definition, and its revision in one read.
    pub fn snapshot(&self, id: &str) -> McpManagementResult<(McpConnectionDetail, Value)> {
        let global = self.load(McpConnectionScope::Global)?;
        let workspace = self.load(McpConnectionScope::Workspace)?;
        let catalog = self.catalog(&global, &workspace)?;
        let summary = catalog
            .connections
            .into_iter()
            .find(|item| item.id == id)
            .ok_or_else(McpManagementError::not_found)?;
        let layer = if summary.scope == McpConnectionScope::Global {
            &global
        } else {
            &workspace
        };
        let raw = servers(&layer.document)
            .and_then(|map| map.get(&summary.name))
            .cloned()
            .ok_or_else(McpManagementError::not_found)?;
        let (config, secret_paths) = redact_connection_config(&raw);
        Ok((
            McpConnectionDetail {
                summary,
                config,
                secret_paths,
                protocol_version: None,
                server_info: None,
                capabilities: None,
                oauth_health: None,
            },
            raw,
        ))
    }

    pub fn get(&self, id: &str) -> McpManagementResult<McpConnectionDetail> {
        self.snapshot(id).map(|(detail, _)| detail)
    }

    pub fn raw(&self, id: &str) -> McpManagementResult<(McpConnectionScope, String, Value)> {
        let (scope, name) = decode_id(id)?;
        let layer = self.load(scope)?;
        let value = servers(&layer.document)
            .and_then(|s| s.get(&name))
            .cloned()
            .ok_or_else(McpManagementError::not_found)?;
        Ok((scope, name, value))
    }

    pub fn resolved(&self, id: &str) -> McpManagementResult<(String, McpServerConfig)> {
        let (detail, value) = self.snapshot(id)?;
        if !detail.summary.effective {
            return Err(McpManagementError::invalid(
                "This connection is overridden by its workspace copy.",
            ));
        }
        let name = detail.summary.name;
        Ok((name.clone(), validate_connection(&name, value)?))
    }

    pub fn create(&self, request: McpConnectionCreateRequest) -> McpManagementResult<String> {
        let name = request.name.trim().to_owned();
        let value = validated_value(&name, request.config)?;
        let layer = self.load(request.scope)?;
        let mut record = self.editable_record(&layer, request.scope)?;
        let map = record_servers_mut(&mut record)?;
        if map.contains_key(&name) {
            return Err(McpManagementError::conflict(
                "A connection with this name already exists in the selected scope.",
            ));
        }
        map.insert(name.clone(), value);
        self.write_record(request.scope, &layer, record)?;
        Ok(connection_id(request.scope, &name))
    }

    pub fn prepare_update(
        &self,
        id: &str,
        request: &McpConnectionUpdateRequest,
    ) -> McpManagementResult<(McpConnectionScope, String, McpServerConfig)> {
        let (scope, name, current) = self.raw(id)?;
        require_revision(&current, &request.expected_revision)?;
        let mut value = request.config.clone();
        restore_secrets(&mut value, &current, &request.preserve_secrets)?;
        let config = validate_connection(&name, value)?;
        Ok((scope, name, config))
    }

    pub fn prepare_import(
        &self,
        request: &McpImportRequest,
    ) -> McpManagementResult<Vec<(String, McpServerConfig)>> {
        let layer = self.load(request.scope)?;
        if request.expected_revision != import_revision(&request.document, &layer.revision) {
            return Err(McpManagementError::conflict(
                "The import source or destination changed. Preview the import again.",
            ));
        }
        let values = parse_import(&request.document)?;
        let selected: BTreeSet<_> = request.selected.iter().collect();
        if selected.is_empty() {
            return Err(McpManagementError::invalid(
                "Select at least one connection to import.",
            ));
        }
        selected
            .into_iter()
            .map(|name| {
                let value = values.get(name).ok_or_else(|| {
                    McpManagementError::invalid("An import selection is not present in the source.")
                })?;
                if servers(&layer.document).is_some_and(|s| s.contains_key(name))
                    && !request.replace
                {
                    return Err(McpManagementError::conflict(
                        "A selected connection already exists. Explicitly allow replacement.",
                    ));
                }
                Ok((name.clone(), validate_connection(name, value.clone())?))
            })
            .collect()
    }

    pub fn update(&self, id: &str, request: McpConnectionUpdateRequest) -> McpManagementResult<()> {
        let (scope, name) = decode_id(id)?;
        let layer = self.load(scope)?;
        let mut record = self.editable_record(&layer, scope)?;
        let map = record_servers_mut(&mut record)?;
        let current = map.get(&name).ok_or_else(McpManagementError::not_found)?;
        require_revision(current, &request.expected_revision)?;
        let mut value = request.config;
        restore_secrets(&mut value, current, &request.preserve_secrets)?;
        map.insert(name.clone(), validated_value(&name, value)?);
        self.write_record(scope, &layer, record)
    }

    pub fn remove(&self, id: &str, expected_revision: &str) -> McpManagementResult<()> {
        let (scope, name) = decode_id(id)?;
        let layer = self.load(scope)?;
        let mut record = self.editable_record(&layer, scope)?;
        let map = record_servers_mut(&mut record)?;
        require_revision(
            map.get(&name).ok_or_else(McpManagementError::not_found)?,
            expected_revision,
        )?;
        map.remove(&name);
        self.write_record(scope, &layer, record)
    }

    pub fn set_enabled(
        &self,
        id: &str,
        request: McpConnectionEnabledRequest,
    ) -> McpManagementResult<()> {
        let (_, _, mut config) = self.raw(id)?;
        config
            .as_object_mut()
            .ok_or_else(|| {
                McpManagementError::invalid(
                    "Repair this connection's configuration before enabling it.",
                )
            })?
            .insert("enabled".into(), Value::Bool(request.enabled));
        self.update(
            id,
            McpConnectionUpdateRequest {
                config,
                preserve_secrets: vec![],
                expected_revision: request.expected_revision,
            },
        )
    }

    pub fn set_bridge_enabled(&self, request: McpBridgeEnabledRequest) -> McpManagementResult<()> {
        let layer = self.load(request.scope)?;
        let mut record = self.editable_record(&layer, request.scope)?;
        record["enabled"] = Value::Bool(request.enabled);
        self.write_record(request.scope, &layer, record)
    }

    fn editable_record(
        &self,
        layer: &Layer,
        scope: McpConnectionScope,
    ) -> McpManagementResult<Value> {
        let mut value = match record(&layer.document) {
            Some(value) => value.clone(),
            None => {
                let enabled = if scope == McpConnectionScope::Workspace {
                    record(&self.load(McpConnectionScope::Global)?.document)
                        .and_then(|r| r.get("enabled"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
                } else {
                    true
                };
                json!({"enabled": enabled, "package":{"kind":"static"},"settings":{"servers":{}}})
            }
        };
        if value
            .pointer("/package/kind")
            .and_then(Value::as_str)
            .unwrap_or("static")
            != "static"
        {
            return Err(McpManagementError::invalid(
                "agena.mcp is replaced by an external plugin in this scope. Restore the built-in bridge in Plugins & Tools before editing these connections.",
            ));
        }
        let obj = value.as_object_mut().ok_or_else(|| {
            McpManagementError::invalid("The MCP plugin record must be an object.")
        })?;
        obj.entry("package")
            .or_insert_with(|| json!({"kind":"static"}));
        if obj.get("settings").is_none_or(Value::is_null) {
            obj.insert("settings".into(), json!({}));
        }
        Ok(value)
    }

    pub fn preview_import(
        &self,
        request: &McpImportPreviewRequest,
    ) -> McpManagementResult<McpImportPreview> {
        let layer = self.load(request.scope)?;
        let parsed = parse_import(&request.document)?;
        let candidates = parsed
            .into_iter()
            .map(|(name, value)| {
                let result = validated_value(&name, value.clone());
                let (config, secret_paths) = result
                    .as_ref()
                    .map(redact_connection_config)
                    .unwrap_or_else(|_| (json!({}), vec![]));
                McpImportCandidate {
                    conflict: servers(&layer.document).is_some_and(|s| s.contains_key(&name)),
                    name,
                    config,
                    secret_paths,
                    valid: result.is_ok(),
                    errors: result.err().map(|e| vec![e.message]).unwrap_or_default(),
                }
            })
            .collect();
        Ok(McpImportPreview {
            revision: import_revision(&request.document, &layer.revision),
            candidates,
            errors: vec![],
        })
    }

    pub fn import(&self, request: McpImportRequest) -> McpManagementResult<Vec<String>> {
        let layer = self.load(request.scope)?;
        if request.expected_revision != import_revision(&request.document, &layer.revision) {
            return Err(McpManagementError::conflict(
                "The import source or destination changed. Preview the import again.",
            ));
        }
        let values = parse_import(&request.document)?;
        let selected: BTreeSet<_> = request.selected.into_iter().collect();
        if selected.is_empty() {
            return Err(McpManagementError::invalid(
                "Select at least one connection to import.",
            ));
        }
        let mut record = self.editable_record(&layer, request.scope)?;
        let map = record_servers_mut(&mut record)?;
        let mut ids = Vec::new();
        for name in selected {
            let value = values.get(&name).ok_or_else(|| {
                McpManagementError::invalid("An import selection is not present in the source.")
            })?;
            if map.contains_key(&name) && !request.replace {
                return Err(McpManagementError::conflict(
                    "Some selected connections already exist. Explicitly allow replacement or change the selection.",
                ));
            }
            map.insert(name.clone(), validated_value(&name, value.clone())?);
            ids.push(connection_id(request.scope, &name));
        }
        self.write_record(request.scope, &layer, record)?;
        Ok(ids)
    }
}

fn record(doc: &Value) -> Option<&Value> {
    doc.pointer(RECORD_POINTER)
}
fn servers(doc: &Value) -> Option<&serde_json::Map<String, Value>> {
    record(doc)?.pointer("/settings/servers")?.as_object()
}
fn record_servers_mut(
    record: &mut Value,
) -> McpManagementResult<&mut serde_json::Map<String, Value>> {
    let settings = record
        .get_mut("settings")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| McpManagementError::invalid("MCP settings must be an object."))?;
    settings
        .entry("servers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| McpManagementError::invalid("MCP servers must be an object."))
}
pub fn connection_revision(value: &Value) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(value).unwrap_or_default(),
    ))
}
fn import_revision(doc: &Value, revision: &str) -> String {
    connection_revision(&json!([doc, revision]))
}
fn require_revision(value: &Value, expected: &str) -> McpManagementResult<()> {
    if expected.is_empty() || expected != connection_revision(value) {
        return Err(McpManagementError::conflict(
            "This connection changed after it was opened. Refresh before saving or deleting.",
        ));
    }
    Ok(())
}
pub fn connection_id(scope: McpConnectionScope, name: &str) -> String {
    format!(
        "{}:{}",
        if scope == McpConnectionScope::Global {
            "global"
        } else {
            "workspace"
        },
        hex::encode(name.as_bytes())
    )
}
fn decode_id(id: &str) -> McpManagementResult<(McpConnectionScope, String)> {
    let (scope, encoded) = id
        .split_once(':')
        .ok_or_else(McpManagementError::not_found)?;
    let scope = match scope {
        "global" => McpConnectionScope::Global,
        "workspace" => McpConnectionScope::Workspace,
        _ => return Err(McpManagementError::not_found()),
    };
    let name =
        String::from_utf8(hex::decode(encoded).map_err(|_| McpManagementError::not_found())?)
            .map_err(|_| McpManagementError::not_found())?;
    if connection_id(scope, &name) != id {
        return Err(McpManagementError::not_found());
    }
    Ok((scope, name))
}
pub fn validate_connection(name: &str, value: Value) -> McpManagementResult<McpServerConfig> {
    if has_redacted(&value) {
        return Err(McpManagementError::invalid(
            "A hidden value must be preserved explicitly or replaced before saving.",
        ));
    }
    let config: McpServerConfig = serde_json::from_value(value).map_err(|_| McpManagementError::invalid("Invalid MCP transport configuration. Check required fields, field names, and value types."))?;
    config.validate(name).map_err(McpManagementError::invalid)?;
    Ok(config)
}
fn validated_value(name: &str, value: Value) -> McpManagementResult<Value> {
    serde_json::to_value(validate_connection(name, value)?)
        .map_err(|_| McpManagementError::internal("Could not encode the connection configuration."))
}
fn summary(
    scope: McpConnectionScope,
    name: &str,
    config: &Value,
    effective: bool,
    bridge_enabled: bool,
) -> McpConnectionSummary {
    let error = validate_connection(name, config.clone())
        .err()
        .map(|e| e.message);
    let enabled = config
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let state = if error.is_some() {
        "invalid"
    } else if !effective {
        "shadowed"
    } else if !enabled {
        "disabled"
    } else if !bridge_enabled {
        "bridge_disabled"
    } else {
        "disconnected"
    };
    McpConnectionSummary {
        id: connection_id(scope, name),
        name: name.into(),
        scope,
        transport: config
            .get("transport")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        enabled,
        effective,
        state: state.into(),
        tool_count: 0,
        revision: connection_revision(config),
        error,
    }
}
fn has_redacted(value: &Value) -> bool {
    match value {
        Value::String(s) => s == REDACTED,
        Value::Array(a) => a.iter().any(has_redacted),
        Value::Object(o) => o.values().any(has_redacted),
        _ => false,
    }
}
pub fn redact_connection_config(value: &Value) -> (Value, Vec<String>) {
    let mut safe = value.clone();
    let mut paths = Vec::new();
    for parent in [
        "/process/env",
        "/endpoint/headers",
        "/auth/headers",
        "/env",
        "/headers",
    ] {
        if let Some(map) = safe.pointer_mut(parent).and_then(Value::as_object_mut) {
            for (name, value) in map {
                paths.push(format!(
                    "{parent}/{}",
                    name.replace('~', "~0").replace('/', "~1")
                ));
                *value = Value::String(REDACTED.into());
            }
        }
    }
    for pointer in [
        "/auth/token",
        "/auth/client_secret",
        "/endpoint/url",
        "/auth/client_metadata_url",
    ] {
        if let Some(value) = safe.pointer_mut(pointer) {
            let secret = !pointer.ends_with("url")
                || value.as_str().is_some_and(|s| {
                    s.contains(['?', '#'])
                        || url::Url::parse(s)
                            .ok()
                            .is_some_and(|u| !u.username().is_empty() || u.password().is_some())
                });
            if secret {
                *value = Value::String(REDACTED.into());
                paths.push(pointer.into());
            }
        }
    }
    paths.sort();
    (safe, paths)
}
fn restore_secrets(
    value: &mut Value,
    current: &Value,
    preserve: &[String],
) -> McpManagementResult<()> {
    let (_, allowed) = redact_connection_config(current);
    for path in preserve {
        if !allowed.contains(path) {
            return Err(McpManagementError::invalid(
                "An unrecognized secret preservation path was supplied.",
            ));
        }
        let existing = current
            .pointer(path)
            .ok_or_else(|| McpManagementError::invalid("The preserved secret no longer exists."))?;
        let slot = value.pointer_mut(path).ok_or_else(|| {
            McpManagementError::invalid(
                "A preserved secret field is missing from the submitted configuration.",
            )
        })?;
        *slot = existing.clone();
    }
    Ok(())
}
fn parse_import(doc: &Value) -> McpManagementResult<serde_json::Map<String, Value>> {
    if serde_json::to_vec(doc).map_or(true, |b| b.len() > MAX_IMPORT_BYTES) {
        return Err(McpManagementError::invalid(
            "MCP imports must be at most 4 MiB.",
        ));
    }
    let input = doc
        .get("mcpServers")
        .or_else(|| doc.get("servers"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            McpManagementError::invalid("Import an object containing mcpServers or servers.")
        })?;
    if input.len() > MAX_IMPORT_CONNECTIONS {
        return Err(McpManagementError::invalid(
            "Import at most 200 connections at once.",
        ));
    }
    let mut output = serde_json::Map::new();
    for (name, raw) in input {
        let value = if raw.get("transport").is_some() {
            raw.clone()
        } else if let Some(obj) = raw.as_object() {
            let allowed: &[&str] = if obj.contains_key("command") {
                &[
                    "command", "args", "env", "cwd", "type", "disabled", "enabled", "tools",
                    "timeouts",
                ]
            } else {
                &[
                    "url", "headers", "type", "disabled", "enabled", "auth", "tools", "timeouts",
                ]
            };
            let unsupported = obj.keys().any(|k| !allowed.contains(&k.as_str()))
                || obj.get("type").is_some_and(|v| !v.is_string())
                || obj.get("disabled").is_some_and(|v| !v.is_boolean())
                || (obj.contains_key("disabled") && obj.contains_key("enabled"));
            if unsupported {
                raw.clone()
            } else if obj.contains_key("url")
                && !obj.contains_key("command")
                && obj
                    .get("type")
                    .and_then(Value::as_str)
                    .is_none_or(|t| t == "http" || t == "streamable-http")
            {
                let mut value = json!({"transport":"http","endpoint":{"url":obj.get("url"),"headers":obj.get("headers").cloned().unwrap_or_else(|| json!({}))},"enabled": !obj.get("disabled").and_then(Value::as_bool).unwrap_or(false)});
                for key in ["auth", "tools", "timeouts", "enabled"] {
                    if let Some(v) = obj.get(key) {
                        value[key] = v.clone();
                    }
                }
                value
            } else if obj.contains_key("command")
                && !obj.contains_key("url")
                && obj
                    .get("type")
                    .and_then(Value::as_str)
                    .is_none_or(|t| t == "stdio")
            {
                let mut process = json!({"command":obj.get("command")});
                for key in ["args", "env", "cwd"] {
                    if let Some(v) = obj.get(key) {
                        process[key] = v.clone();
                    }
                }
                let mut value = json!({"transport":"stdio","process":process,"enabled":!obj.get("disabled").and_then(Value::as_bool).unwrap_or(false)});
                for key in ["tools", "timeouts", "enabled"] {
                    if let Some(v) = obj.get(key) {
                        value[key] = v.clone();
                    }
                }
                value
            } else {
                raw.clone()
            }
        } else {
            raw.clone()
        };
        output.insert(name.clone(), value);
    }
    Ok(output)
}

/// MCP scopes merge individual complete server definitions; other plugin
/// records retain their existing whole-record replacement semantics.
pub fn merge_mcp_settings(base: &Value, overlay: &Value) -> Value {
    let mut result = base.as_object().cloned().unwrap_or_default();
    if let Some(map) = overlay.as_object() {
        for (key, value) in map {
            if key == "servers" {
                let mut entries = result
                    .get(key)
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                if let Some(overrides) = value.as_object() {
                    entries.extend(overrides.clone());
                    result.insert(key.clone(), Value::Object(entries));
                } else {
                    result.insert(key.clone(), value.clone());
                }
            } else if key == "runtime" {
                let mut current = result.get(key).cloned().unwrap_or_else(|| json!({}));
                merge_objects(&mut current, value);
                result.insert(key.clone(), current);
            } else {
                result.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(result)
}
fn merge_objects(target: &mut Value, overlay: &Value) {
    match (target, overlay) {
        (Value::Object(target), Value::Object(overlay)) => {
            for (key, value) in overlay {
                merge_objects(target.entry(key).or_insert(Value::Null), value);
            }
        }
        (target, overlay) => *target = overlay.clone(),
    }
}
