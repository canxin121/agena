//! Loader for one configured plugin.

use serde_json::Value as JsonValue;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::config::{ConfiguredPlugin, PluginPackage, PluginSignature};
use crate::error::{HostError, TransportError};
use crate::host::{HostHandle, LoadedPlugin};
use crate::registry::validate_tool_definition;
use crate::sdk::rpc::method;
use crate::sdk::{InitContext, InitOutcome, PluginKey, PluginManifest};
use crate::transport::{
    PluginTransport, cdylib::CdylibTransport, http::HttpTransport, stdio::StdioTransport,
};

const TRANSPORT_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSPORT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

async fn dispatch_transport_with_timeout(
    transport: &dyn PluginTransport,
    method: &str,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, TransportError> {
    tokio::time::timeout(timeout, transport.dispatch(method, params))
        .await
        .map_err(|_| TransportError::Timeout)?
}

/// Static plugin registration entry.
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
        let _ = tokio::time::timeout(TRANSPORT_SHUTDOWN_TIMEOUT, transport.close()).await;
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

    let prefetched_manifest_value = dispatch_transport_with_timeout(
        transport.as_ref(),
        method::META_MANIFEST,
        serde_json::Value::Object(Default::default()),
        TRANSPORT_INITIALIZATION_TIMEOUT,
    )
    .await
    .map_err(|error| HostError::Init {
        plugin: plugin_id.to_string(),
        message: match error {
            TransportError::Timeout => "meta/manifest timed out".to_string(),
            error => error.to_string(),
        },
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

    let outcome_value = dispatch_transport_with_timeout(
        transport.as_ref(),
        method::META_INIT,
        init_params,
        TRANSPORT_INITIALIZATION_TIMEOUT,
    )
    .await
    .map_err(|error| HostError::Init {
        plugin: plugin_id.to_string(),
        message: match error {
            TransportError::Timeout => "meta/init timed out".to_string(),
            error => error.to_string(),
        },
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

    let mut skill_names = BTreeSet::new();
    let mut skill_lookup_names = BTreeSet::new();
    for skill in &manifest.skills {
        validate_id(skill.name.as_str(), "skill", &mut skill_names)?;
        if skill.instructions.chars().count() > 131_072 {
            return Err(fail(format!(
                "skill '{}' instructions exceed the 131072-character manifest limit",
                skill.name
            )));
        }
        if skill.description.chars().count() > 8_192 {
            return Err(fail(format!(
                "skill '{}' description exceeds the 8192-character manifest limit",
                skill.name
            )));
        }
        let canonical = skill.name.to_ascii_lowercase();
        if !skill_lookup_names.insert(canonical) {
            return Err(fail(format!(
                "duplicate skill lookup name '{}'",
                skill.name
            )));
        }
        for alias in &skill.aliases {
            if alias.trim().is_empty() || alias.trim() != alias {
                return Err(fail(format!(
                    "skill '{}' alias '{alias}' must be non-empty and must not contain leading or trailing whitespace",
                    skill.name
                )));
            }
            if !skill_lookup_names.insert(alias.to_ascii_lowercase()) {
                return Err(fail(format!("duplicate skill lookup name '{alias}'")));
            }
        }
    }

    let mut command_ids = BTreeSet::new();
    let mut action_ids = BTreeSet::new();
    for command in &manifest.commands {
        validate_id(command.id.as_str(), "command", &mut command_ids)?;
        validate_id(command.id.as_str(), "studio action", &mut action_ids)?;
        if let crate::sdk::PluginUiAction::OpenPluginWorkbench { tab: Some(tab) } = &command.action
            && !crate::sdk::plugin_workbench_tab_id_is_supported(tab)
        {
            return Err(fail(format!(
                "command '{}' requests unsupported Plugin Workbench tab '{tab}'",
                command.id
            )));
        }
    }

    let mut display_ids = BTreeSet::new();
    for contribution in &manifest.ui.display {
        validate_id(
            contribution.id.as_str(),
            "display contribution",
            &mut display_ids,
        )?;
    }
    let mut theme_ids = BTreeSet::new();
    for theme in &manifest.ui.tui.themes {
        validate_id(theme.id.as_str(), "theme", &mut theme_ids)?;
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

/// Validate a JSON value with the standards-compliant JSON Schema engine used
/// by Agena plugin manifests and generated tool contracts.
///
/// This is intentionally the same validator used for plugin configuration so
/// Tool API callers can reject definitely invalid execution-tool input before
/// the tool handler runs. Agena extension aliases (`x-agena-aliases`) are
/// accepted wherever their canonical property is accepted.
pub fn validate_json_schema_value(schema: &JsonValue, value: &JsonValue) -> Result<(), String> {
    let mut normalized_schema = schema.clone();
    expand_agena_property_aliases(&mut normalized_schema);
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&normalized_schema)
        .map_err(|error| format!("invalid JSON Schema: {error}"))?;
    let Some(error) = validator.iter_errors(value).next() else {
        return Ok(());
    };
    let instance_path = error.instance_path().to_string();
    let path = if instance_path.is_empty() {
        "$".to_owned()
    } else {
        format!("${instance_path}")
    };
    Err(format!("{path}: {error}"))
}

/// Convert Agena's property-alias annotation into ordinary JSON Schema before
/// compiling it with the standards-compliant validator. Alias properties keep
/// the canonical property's complete schema, and a required canonical field is
/// rewritten as an `anyOf` requirement covering its declared aliases.
fn expand_agena_property_aliases(schema: &mut JsonValue) {
    let JsonValue::Object(object) = schema else {
        if let JsonValue::Array(items) = schema {
            for item in items {
                expand_agena_property_aliases(item);
            }
        }
        return;
    };

    for child in object.values_mut() {
        expand_agena_property_aliases(child);
    }

    let aliases = object
        .get("properties")
        .and_then(JsonValue::as_object)
        .map(|properties| {
            properties
                .iter()
                .filter_map(|(canonical, property_schema)| {
                    let aliases = property_schema
                        .get("x-agena-aliases")
                        .and_then(JsonValue::as_array)?
                        .iter()
                        .filter_map(JsonValue::as_str)
                        .filter(|alias| !alias.is_empty() && *alias != canonical)
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>();
                    (!aliases.is_empty())
                        .then(|| (canonical.clone(), aliases, property_schema.clone()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if aliases.is_empty() {
        return;
    }

    if let Some(properties) = object
        .get_mut("properties")
        .and_then(JsonValue::as_object_mut)
    {
        for (_, aliases, property_schema) in &aliases {
            for alias in aliases {
                properties
                    .entry(alias.clone())
                    .or_insert_with(|| property_schema.clone());
            }
        }
    }

    let required = object
        .get("required")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if required.is_empty() {
        return;
    }

    let mut retained = Vec::with_capacity(required.len());
    let mut alias_requirements = Vec::new();
    for required_property in required {
        let Some(canonical) = required_property.as_str() else {
            retained.push(required_property);
            continue;
        };
        let Some((_, property_aliases, _)) = aliases
            .iter()
            .find(|(property, _, _)| property == canonical)
        else {
            retained.push(required_property);
            continue;
        };
        let variants = std::iter::once(canonical)
            .chain(property_aliases.iter().map(String::as_str))
            .map(|property| serde_json::json!({ "required": [property] }))
            .collect::<Vec<_>>();
        alias_requirements.push(serde_json::json!({ "anyOf": variants }));
    }

    if retained.is_empty() {
        object.remove("required");
    } else {
        object.insert("required".to_owned(), JsonValue::Array(retained));
    }
    if !alias_requirements.is_empty() {
        let all_of = object
            .entry("allOf")
            .or_insert_with(|| JsonValue::Array(Vec::new()));
        if let JsonValue::Array(all_of) = all_of {
            all_of.extend(alias_requirements);
        }
    }
}

pub async fn shutdown_transport(transport: Arc<dyn PluginTransport>) -> Result<(), TransportError> {
    let _ = dispatch_transport_with_timeout(
        transport.as_ref(),
        method::META_SHUTDOWN,
        serde_json::Value::Object(Default::default()),
        TRANSPORT_SHUTDOWN_TIMEOUT,
    )
    .await;
    tokio::time::timeout(TRANSPORT_SHUTDOWN_TIMEOUT, transport.close())
        .await
        .map_err(|_| TransportError::Timeout)?
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
    use std::time::Duration;

    use async_trait::async_trait;

    use super::{dispatch_transport_with_timeout, validate_json_schema_value, validate_manifest};
    use crate::error::TransportError;
    use crate::sdk::{
        PluginCommandDefinition, PluginKey, PluginManifest, PluginSkillDefinition,
        PluginStudioControl, ToolDefinition,
    };
    use crate::transport::PluginTransport;

    struct SilentTransport;

    #[async_trait]
    impl PluginTransport for SilentTransport {
        async fn dispatch(
            &self,
            _method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, TransportError> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn silent_plugin_initialization_is_bounded_by_a_deadline() {
        let error = dispatch_transport_with_timeout(
            &SilentTransport,
            "meta/manifest",
            serde_json::Value::Null,
            Duration::from_millis(20),
        )
        .await
        .expect_err("silent transport must time out");
        assert!(matches!(error, TransportError::Timeout));
    }

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
    fn manifest_validation_rejects_plugin_defined_workbench_tabs() {
        let mut manifest = manifest_with_tools(&[]);
        manifest.commands.push(
            serde_json::from_value::<PluginCommandDefinition>(serde_json::json!({
                "id": "open",
                "title": "Open",
                "action": {
                    "kind": "open_plugin_workbench",
                    "tab": "plugin-defined-page"
                }
            }))
            .expect("command definition"),
        );

        assert!(validation_error(&manifest).contains("unsupported Plugin Workbench tab"));
    }

    #[test]
    fn manifest_validation_accepts_plugin_skills_and_rejects_ambiguous_aliases() {
        let mut manifest = manifest_with_tools(&[]);
        manifest.skills = vec![
            PluginSkillDefinition {
                name: "docs".to_string(),
                instructions: "Read the package docs.".to_string(),
                aliases: vec!["plugin-docs".to_string()],
                ..PluginSkillDefinition::default()
            },
            PluginSkillDefinition {
                name: "review".to_string(),
                instructions: "Review the package change.".to_string(),
                ..PluginSkillDefinition::default()
            },
        ];
        validate_manifest(
            "example.plugin",
            &PluginKey::new("example", "plugin").expect("plugin key"),
            &manifest,
            "test",
        )
        .expect("valid plugin skills");

        manifest.skills[1].aliases.push("PLUGIN-DOCS".to_string());
        assert!(validation_error(&manifest).contains("duplicate skill lookup name"));
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
        assert!(error.contains("shorter than 1 character"));
        let error = validate_json_schema_value(&schema, &serde_json::json!({}))
            .expect_err("missing canonical property and aliases must fail");
        assert!(error.contains("not valid under any"));
    }
}
