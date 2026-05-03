//! Per-entry loader: build a `LoadedPlugin` for one config entry.

use std::path::Path;
use std::sync::Arc;

use crate::config::PluginEntry;
use crate::error::{HostError, TransportError};
use crate::host::{HostHandle, LoadedPlugin};
use crate::sdk::rpc::method;
use crate::sdk::{InitContext, InitOutcome, PluginManifest};
use crate::transport::{
    PluginTransport, cdylib::CdylibTransport, http::HttpTransport, stdio::StdioTransport,
};

pub struct StaticRegistration {
    pub builder: Box<dyn FnOnce() -> Arc<dyn PluginTransport> + Send + Sync>,
}

#[allow(clippy::too_many_arguments)]
pub async fn load_entry(
    plugin_id: &str,
    entry: &PluginEntry,
    static_registry: &mut std::collections::HashMap<String, StaticRegistration>,
    host_handle: Arc<HostHandle>,
    agena_version: &str,
    workspace_root: &Path,
    env_lookup: &(dyn Fn(&str) -> Option<String> + Send + Sync),
    trusted_keys: &std::collections::BTreeMap<String, String>,
) -> Result<LoadedPlugin, HostError> {
    let transport: Arc<dyn PluginTransport> = match entry {
        PluginEntry::Static { .. } => {
            let registration =
                static_registry
                    .remove(plugin_id)
                    .ok_or_else(|| HostError::Load {
                        plugin: plugin_id.to_string(),
                        message: format!("no static plugin registered with id `{plugin_id}`"),
                    })?;
            (registration.builder)()
        }
        PluginEntry::Cdylib {
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
        PluginEntry::Stdio {
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
            let t = StdioTransport::spawn_with_policy_and_status(
                command,
                args,
                &env_map,
                cwd.as_ref(),
                Some(host_handler),
                restart.clone(),
                Some(plugin_id.to_string()),
                Some(status_sink),
            )
            .await
            .map_err(|e| HostError::Load {
                plugin: plugin_id.to_string(),
                message: e.to_string(),
            })?;
            Arc::new(t)
        }
        PluginEntry::Http { url, auth, .. } => {
            let t = HttpTransport::new(url.clone(), auth.clone(), env_lookup);
            Arc::new(t)
        }
        #[cfg(feature = "wasm")]
        PluginEntry::Wasm { path, sha256, .. } => {
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
        PluginEntry::Wasm { .. } => {
            return Err(HostError::Load {
                plugin: plugin_id.to_string(),
                message: "wasm transport requires the `wasm` feature".into(),
            });
        }
    };

    transport
        .attach_host(host_handle.scoped_host_client(plugin_id.to_string()))
        .await
        .map_err(|e| HostError::Load {
            plugin: plugin_id.to_string(),
            message: e.to_string(),
        })?;

    let init_ctx = InitContext {
        agena_version: agena_version.to_string(),
        workspace_root: workspace_root.to_path_buf(),
        plugin_id: plugin_id.to_string(),
        host_callback_url: host_handle.callback_url(plugin_id),
        host_callback_token: host_handle.callback_token(plugin_id),
        options: entry.options().clone(),
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

    validate_manifest_options(plugin_id, &outcome.manifest, entry.options())?;

    Ok(LoadedPlugin::new(
        plugin_id.to_string(),
        entry.kind_str(),
        transport,
        outcome.manifest,
    ))
}

#[allow(clippy::result_large_err)]
fn validate_manifest_options(
    plugin_id: &str,
    manifest: &PluginManifest,
    options: &serde_json::Value,
) -> Result<(), HostError> {
    let Some(schema) = manifest.options_schema.as_ref() else {
        return Ok(());
    };
    validate_schema_value("$", schema, options).map_err(|message| {
        HostError::Config(format!(
            "plugin `{plugin_id}` options do not match manifest schema: {message}"
        ))
    })
}

fn validate_schema_value(
    path: &str,
    schema: &serde_json::Value,
    value: &serde_json::Value,
) -> Result<(), String> {
    match schema {
        serde_json::Value::Bool(true) => return Ok(()),
        serde_json::Value::Bool(false) => {
            return Err(format!("{path}: schema rejects this value"));
        }
        serde_json::Value::Object(_) => {}
        _ => return Ok(()),
    }

    let schema_obj = schema.as_object().expect("object already checked");

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

    if let Some(required) = schema_obj.get("required").and_then(|v| v.as_array()) {
        let object = value
            .as_object()
            .ok_or_else(|| format!("{path}: required fields require an object value"))?;
        for field in required {
            let Some(field) = field.as_str() else {
                continue;
            };
            if !object.contains_key(field) {
                return Err(format!("{path}: missing required property '{field}'"));
            }
        }
    }

    if let Some(properties) = schema_obj.get("properties").and_then(|v| v.as_object())
        && let Some(object) = value.as_object()
    {
        for (key, subschema) in properties {
            if let Some(child) = object.get(key) {
                validate_schema_value(&format!("{path}.{key}"), subschema, child)?;
            }
        }
    }

    if let Some(additional) = schema_obj.get("additionalProperties")
        && let Some(object) = value.as_object()
    {
        let properties = schema_obj.get("properties").and_then(|v| v.as_object());
        for (key, child) in object {
            if properties.is_some_and(|props| props.contains_key(key)) {
                continue;
            }
            match additional {
                serde_json::Value::Bool(true) => {}
                serde_json::Value::Bool(false) => {
                    return Err(format!("{path}: unexpected property '{key}'"));
                }
                other => validate_schema_value(&format!("{path}.{key}"), other, child)?,
            }
        }
    }

    if let Some(item_schema) = schema_obj.get("items")
        && let Some(items) = value.as_array()
    {
        for (index, item) in items.iter().enumerate() {
            validate_schema_value(&format!("{path}[{index}]"), item_schema, item)?;
        }
    }

    if let Some(min_length) = schema_obj.get("minLength").and_then(|v| v.as_u64())
        && let Some(text) = value.as_str()
        && text.chars().count() < min_length as usize
    {
        return Err(format!(
            "{path}: string is shorter than minLength {}",
            min_length
        ));
    }

    Ok(())
}

fn validate_schema_type(
    path: &str,
    expected: &serde_json::Value,
    value: &serde_json::Value,
) -> Result<(), String> {
    let matches = match expected {
        serde_json::Value::String(kind) => value_matches_type(kind, value),
        serde_json::Value::Array(kinds) => kinds
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

fn value_matches_type(kind: &str, value: &serde_json::Value) -> bool {
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

pub async fn shutdown_transport(transport: Arc<dyn PluginTransport>) -> Result<(), TransportError> {
    let _ = transport
        .dispatch(
            method::META_SHUTDOWN,
            serde_json::Value::Object(Default::default()),
        )
        .await;
    transport.close().await
}

#[allow(dead_code)]
pub fn manifest_summary(m: &PluginManifest) -> String {
    format!("{}@{}", m.name, m.version)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::validate_schema_value;

    #[test]
    fn schema_validator_accepts_matching_object() {
        let schema = json!({
            "type": "object",
            "required": ["token"],
            "properties": {
                "token": { "type": "string", "minLength": 1 },
                "enabled": { "type": "boolean" }
            },
            "additionalProperties": false
        });
        let value = json!({
            "token": "abc",
            "enabled": true
        });

        validate_schema_value("$", &schema, &value).expect("value should satisfy schema");
    }

    #[test]
    fn schema_validator_rejects_missing_required_property() {
        let schema = json!({
            "type": "object",
            "required": ["token"],
            "properties": {
                "token": { "type": "string" }
            }
        });
        let value = json!({});

        let err = validate_schema_value("$", &schema, &value)
            .expect_err("schema should reject missing required property");
        assert!(err.contains("missing required property 'token'"));
    }

    #[test]
    fn schema_validator_rejects_unexpected_property() {
        let schema = json!({
            "type": "object",
            "properties": {
                "token": { "type": "string" }
            },
            "additionalProperties": false
        });
        let value = json!({
            "token": "abc",
            "extra": true
        });

        let err = validate_schema_value("$", &schema, &value)
            .expect_err("schema should reject unexpected property");
        assert!(err.contains("unexpected property 'extra'"));
    }
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
    let bytes = std::fs::read(path).map_err(|e| format!("read `{}`: {e}", path.display()))?;
    verifier.verify(&bytes, &signature).map_err(|e| {
        format!(
            "signature verification failed for `{}`: {e}",
            path.display()
        )
    })
}
