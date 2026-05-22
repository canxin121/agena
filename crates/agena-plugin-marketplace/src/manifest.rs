//! Marketplace registry index + per-version manifest. Static JSON shape that
//! a plugin author publishes, and a host downloads to decide what to install.

use std::collections::BTreeMap;

use agena_plugin_host::PluginSignature;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistryIndex {
    #[serde(default = "default_index_version")]
    pub version: u32,
    #[serde(default)]
    pub plugins: Vec<PluginRecord>,
}

fn default_index_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<PluginVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginVersion {
    pub version: String,
    pub kind: PluginKind,
    /// rustc target triple or `"any"` for portable artifacts (wasm).
    #[serde(default = "default_platform")]
    pub platform: String,
    /// HTTP(S) URL of the artifact bytes.
    pub url: String,
    /// sha256 of the artifact (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Optional ed25519 signature over the artifact bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<PluginSignature>,
    /// Stdio transports keep their command name; for cdylib/wasm the host
    /// computes the install path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub options: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_agena_version: Option<String>,
    /// When set, the artifact bytes are treated as an archive and extracted
    /// under `plugins/<id>/<version>/` instead of being written verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveSpec>,
    /// Optional dependency list resolved at install time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum ArchiveSpec {
    /// gzip tar archive. The named entrypoint inside the archive is what the
    /// final config's `command`/`path` field will point to.
    TarGz { entrypoint: String },
}

/// A single dependency reference: another plugin id + a semver requirement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencySpec {
    pub plugin_id: String,
    pub version_req: String,
}

fn default_platform() -> String {
    "any".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    Cdylib,
    Stdio,
    Http,
    Wasm,
}

impl PluginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cdylib => "cdylib",
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Wasm => "wasm",
        }
    }

    pub fn artifact_extension(self) -> &'static str {
        match self {
            Self::Cdylib => {
                if cfg!(target_os = "windows") {
                    "dll"
                } else if cfg!(target_os = "macos") {
                    "dylib"
                } else {
                    "so"
                }
            }
            Self::Wasm => "wasm",
            Self::Stdio => "bin",
            Self::Http => "txt",
        }
    }
}
