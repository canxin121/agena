//! Top-level marketplace API: registries, fetch, install, uninstall.

pub mod agent_packages;
pub mod cache;
pub mod error;
pub mod installer;
pub mod manifest;
pub mod project;

#[cfg(test)]
mod web_package_tests;

use std::time::Duration;
use std::{
    collections::BTreeMap,
    io::Read,
    path::Path,
    sync::{Arc, OnceLock},
};

pub use cache::{
    InstalledRecord, InstalledRecords, MarketplaceCache, default_cache_root, write_secure_file,
};
pub use error::MarketplaceError;
pub use installer::{
    DEFAULT_MARKETPLACE_SOURCE, InstallOutcome, InstallRequest, MarketplaceClient, OutdatedRecord,
    PluginInstallLocator, RegistryHandle, RegistrySpec, UninstallOutcome, UpgradeOutcome,
    current_target_triple, parse_plugin_install_locator,
};
pub use manifest::{
    AGENA_MARKETPLACE_FILENAME, AGENA_RELEASE_MANIFEST_FILENAME, ArchiveSpec, DependencySpec,
    MarketplaceMetadata, MarketplaceOwner, MarketplaceReviewTier, PluginKind, PluginRecord,
    PluginReleaseArtifact, PluginReleaseManifest, PluginReleaseSource, PluginSignature,
    PluginVersion, RegistryIndex,
};
pub use project::{
    AGENA_MARKETPLACE_PROJECT_FILENAME, AGENA_PROJECT_MANIFEST_FILENAME,
    AGENA_TEMPLATE_BASELINE_REF, AddMarketplaceReleaseOutcome, AddMarketplaceReleaseRequest,
    AssembleReleaseOutcome, AssembleReleaseRequest, BuildMarketplaceOutcome,
    BuildMarketplaceRequest, MarketplacePluginPolicy, MarketplaceProjectManifest,
    PackagePluginOutcome, PackagePluginRequest, PluginProjectManifest, PluginTemplateKind,
    ScaffoldMarketplaceRequest, ScaffoldPluginRequest, add_marketplace_release, assemble_release,
    build_marketplace, generate_plugin_lockfile, package_plugin, scaffold_marketplace,
    scaffold_plugin,
};

/// Read marketplace Ed25519 keys from Agena's existing plugin host trust
/// configuration (`plugins.host.trusted_keys`). The public keys are not
/// secrets, and sharing this trust store keeps plugin package and marketplace
/// artifact verification under one explicit operator-managed policy.
pub fn trusted_keys_from_config(
    config_path: &Path,
) -> Result<BTreeMap<String, String>, MarketplaceError> {
    if !config_path.exists() {
        return Ok(BTreeMap::new());
    }
    let bytes = std::fs::read(config_path)?;
    let document: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        MarketplaceError::Config(format!(
            "could not read marketplace trust keys from {}: {error}",
            config_path.display()
        ))
    })?;
    let Some(value) = document.pointer("/plugins/host/trusted_keys") else {
        return Ok(BTreeMap::new());
    };
    let Some(keys) = value.as_object() else {
        return Err(MarketplaceError::Config(format!(
            "plugins.host.trusted_keys in {} must be an object of key IDs and hex public keys",
            config_path.display()
        )));
    };
    keys.iter()
        .map(|(key_id, value)| {
            let public_key = value.as_str().ok_or_else(|| {
                MarketplaceError::Config(format!(
                    "trusted marketplace key `{key_id}` in {} must be a hex string",
                    config_path.display()
                ))
            })?;
            Ok((key_id.clone(), public_key.to_owned()))
        })
        .collect()
}

/// Pluggable HTTP fetcher so tests can supply local bytes without a network.
pub trait HttpFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError>;

    fn fetch_bounded(
        &self,
        url: &str,
        limit: u64,
        _timeout: Duration,
    ) -> Result<Vec<u8>, MarketplaceError> {
        let bytes = self.fetch(url)?;
        if bytes.len() as u64 > limit {
            return Err(fetch_size_error(url, limit));
        }
        Ok(bytes)
    }
}

/// Default fetcher: reqwest blocking client. `file://` urls are supported by
/// [`reqwest`] only on some platforms, so we shortcut them here for tests and
/// vendored offline registries.
#[derive(Debug, Clone, Default)]
pub struct ReqwestFetcher {
    client: Arc<OnceLock<Result<reqwest::blocking::Client, String>>>,
}

const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_FETCH_BYTES: u64 = 64 * 1024 * 1024;

impl ReqwestFetcher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl HttpFetcher for ReqwestFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError> {
        self.fetch_bounded(url, MAX_FETCH_BYTES, FETCH_TIMEOUT)
    }

    fn fetch_bounded(
        &self,
        url: &str,
        limit: u64,
        timeout: Duration,
    ) -> Result<Vec<u8>, MarketplaceError> {
        let limit = limit.min(MAX_FETCH_BYTES);
        if let Some(path) = url.strip_prefix("file://") {
            let file = std::fs::File::open(path)?;
            if file.metadata()?.len() > limit {
                return Err(fetch_size_error(path, limit));
            }
            return read_bounded(file, path, limit);
        }
        let client = self
            .client
            .get_or_init(|| {
                reqwest::blocking::Client::builder()
                    .user_agent(concat!("Agena/", env!("CARGO_PKG_VERSION")))
                    .connect_timeout(CONNECT_TIMEOUT)
                    .timeout(FETCH_TIMEOUT)
                    .build()
                    .map_err(|error| agena_failure::diagnostic::format_error_chain(&error))
            })
            .as_ref()
            .map_err(|error| MarketplaceError::Http(error.clone()))?;
        let response = client
            .get(url)
            .timeout(timeout)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| MarketplaceError::Http(format!("GET {url}: {e}")))?;
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err(fetch_size_error(url, limit));
        }
        read_bounded(response, url, limit)
    }
}

fn fetch_size_error(source: &str, limit: u64) -> MarketplaceError {
    MarketplaceError::Http(format!(
        "{source}: response exceeds the {limit}-byte marketplace download limit"
    ))
}

fn read_bounded(reader: impl Read, source: &str, limit: u64) -> Result<Vec<u8>, MarketplaceError> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| MarketplaceError::Http(format!("read body {source}: {error}")))?;
    if bytes.len() as u64 > limit {
        return Err(fetch_size_error(source, limit));
    }
    Ok(bytes)
}
