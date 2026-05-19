use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct MarketplacePluginResource {
    pub plugin_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub version_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_platform: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceSearchResponse {
    pub registry_id: String,
    pub registry_url: String,
    pub entries: Vec<MarketplacePluginResource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceRegistryRequestBody {
    #[serde(default)]
    pub registry_id: Option<String>,
    pub registry_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceSearchRequestBody {
    #[serde(default)]
    pub registry_id: Option<String>,
    pub registry_url: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceSyncResponse {
    pub registry_id: String,
    pub registry_url: String,
    pub plugin_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceInstalledPluginResource {
    pub plugin_id: String,
    pub version: String,
    pub kind: String,
    pub platform: String,
    pub binary_path: String,
    pub config_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub registry_id: String,
    pub registry_url: String,
    pub archive_extracted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceInstalledListResponse {
    pub entries: Vec<MarketplaceInstalledPluginResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceOutdatedPluginResource {
    pub plugin_id: String,
    pub installed_version: String,
    pub latest_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceOutdatedListResponse {
    pub entries: Vec<MarketplaceOutdatedPluginResource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceInstallRequestBody {
    pub spec: String,
    #[serde(default)]
    pub registry_id: Option<String>,
    pub registry_url: String,
    #[serde(default)]
    pub config_path: Option<String>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub allow_unverified: bool,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub require_signature: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceInstallOutcomeResource {
    pub plugin_id: String,
    pub version: String,
    pub kind: String,
    pub artifact_path: String,
    pub config_path: String,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceUninstallRequestBody {
    pub plugin_id: String,
    #[serde(default)]
    pub cascade: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceUninstallOutcomeResource {
    pub plugin_id: String,
    pub version: String,
    pub config_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceUninstallResponse {
    pub entries: Vec<MarketplaceUninstallOutcomeResource>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MarketplaceUpgradeRequestBody {
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub registry_id: Option<String>,
    #[serde(default)]
    pub registry_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceUpgradeOutcomeResource {
    pub plugin_id: String,
    pub previous_version: String,
    pub installed_version: String,
    pub upgraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<MarketplaceInstallOutcomeResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceUpgradeResponse {
    pub entries: Vec<MarketplaceUpgradeOutcomeResource>,
}
