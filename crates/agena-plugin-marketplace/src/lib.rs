//! Top-level marketplace API: registries, fetch, install, uninstall.

pub mod cache;
pub mod error;
pub mod installer;
pub mod manifest;
pub mod project;

use std::io::Read;
use std::time::Duration;

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

/// Pluggable HTTP fetcher so tests can supply local bytes without a network.
pub trait HttpFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError>;
}

/// Default fetcher: reqwest blocking client. `file://` urls are supported by
/// [`reqwest`] only on some platforms, so we shortcut them here for tests and
/// vendored offline registries.
#[derive(Debug, Clone, Default)]
pub struct ReqwestFetcher {
    client: Option<reqwest::blocking::Client>,
}

const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_FETCH_BYTES: usize = 64 * 1024 * 1024;

impl ReqwestFetcher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl HttpFetcher for ReqwestFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError> {
        if let Some(path) = url.strip_prefix("file://") {
            let file = std::fs::File::open(path)?;
            if file.metadata()?.len() > MAX_FETCH_BYTES as u64 {
                return Err(MarketplaceError::Http(format!(
                    "file exceeds the {MAX_FETCH_BYTES}-byte marketplace download limit"
                )));
            }
            return read_bounded(file, path);
        }
        let client = match &self.client {
            Some(client) => client.clone(),
            None => reqwest::blocking::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(FETCH_TIMEOUT)
                .build()
                .map_err(|e| MarketplaceError::Http(e.to_string()))?,
        };
        let response = client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| MarketplaceError::Http(format!("GET {url}: {e}")))?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_FETCH_BYTES as u64)
        {
            return Err(MarketplaceError::Http(format!(
                "GET {url}: response exceeds the {MAX_FETCH_BYTES}-byte marketplace download limit"
            )));
        }
        read_bounded(response, url)
    }
}

fn read_bounded(reader: impl Read, source: &str) -> Result<Vec<u8>, MarketplaceError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_FETCH_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| MarketplaceError::Http(format!("read body {source}: {error}")))?;
    if bytes.len() > MAX_FETCH_BYTES {
        return Err(MarketplaceError::Http(format!(
            "{source}: response exceeds the {MAX_FETCH_BYTES}-byte marketplace download limit"
        )));
    }
    Ok(bytes)
}
