//! Provider-wide model discovery, with route provenance and partial failures.

use agena_domain::{AdapterId, Model};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelDiscoverySource {
    ProviderDiscovery,
    Configured,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredProviderModel {
    #[serde(flatten)]
    pub model: Model,
    pub source: ModelDiscoverySource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdapterDiscoveryStatus {
    Discovered,
    ConfiguredOnly,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterModelDiscovery {
    pub adapter_id: Option<AdapterId>,
    pub status: AdapterDiscoveryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelDiscovery {
    pub models: Vec<DiscoveredProviderModel>,
    pub adapters: Vec<AdapterModelDiscovery>,
    /// All requested remote discoveries succeeded. Configured-only adapters
    /// remain explicitly identified; completeness does not assert API access.
    pub complete: bool,
}

impl ProviderModelDiscovery {
    pub fn from_models(models: Vec<Model>) -> Self {
        Self {
            models: models
                .into_iter()
                .map(|model| DiscoveredProviderModel {
                    model,
                    source: ModelDiscoverySource::ProviderDiscovery,
                })
                .collect(),
            adapters: Vec::new(),
            complete: true,
        }
    }
}
