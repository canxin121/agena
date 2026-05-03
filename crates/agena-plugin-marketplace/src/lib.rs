//! Top-level marketplace API: registries, fetch, install, uninstall.

pub mod cache;
pub mod error;
pub mod installer;
pub mod manifest;

pub use cache::{
    InstalledRecord, InstalledRecords, MarketplaceCache, default_cache_root, write_secure_file,
};
pub use error::MarketplaceError;
pub use installer::{
    InstallOutcome, InstallRequest, MarketplaceClient, RegistryHandle, RegistrySpec,
    UninstallOutcome,
};
pub use manifest::{PluginKind, PluginRecord, PluginVersion, RegistryIndex};

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

impl ReqwestFetcher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl HttpFetcher for ReqwestFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError> {
        if let Some(path) = url.strip_prefix("file://") {
            return Ok(std::fs::read(path)?);
        }
        let client = match &self.client {
            Some(client) => client.clone(),
            None => reqwest::blocking::Client::builder()
                .build()
                .map_err(|e| MarketplaceError::Http(e.to_string()))?,
        };
        let response = client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| MarketplaceError::Http(format!("GET {url}: {e}")))?;
        let bytes = response
            .bytes()
            .map_err(|e| MarketplaceError::Http(format!("read body {url}: {e}")))?;
        Ok(bytes.to_vec())
    }
}
