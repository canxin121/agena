//! Read a `plugin.toml` manifest sitting next to a cdylib/stdio binary, or
//! materialize one from a `meta/manifest` JSON-RPC response.

use std::path::Path;

use crate::error::HostError;
use crate::sdk::PluginManifest;

pub fn read_manifest_toml(path: &Path) -> Result<PluginManifest, HostError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| HostError::Config(format!("read manifest `{}`: {e}", path.display())))?;
    let manifest: PluginManifest = toml::from_str(&raw)
        .map_err(|e| HostError::Config(format!("parse manifest `{}`: {e}", path.display())))?;
    Ok(manifest)
}

pub fn parse_manifest_json(value: serde_json::Value) -> Result<PluginManifest, HostError> {
    serde_json::from_value(value)
        .map_err(|e| HostError::Config(format!("parse manifest from JSON: {e}")))
}
