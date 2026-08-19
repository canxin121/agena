//! Marketplace registry index + per-version manifest. Static JSON shape that
//! a plugin author publishes, and a host downloads to decide what to install.

use std::{collections::BTreeMap, fmt};

use agena_plugin_host::PluginSignature;
use serde::{Deserialize, Serialize};

use crate::MarketplaceError;

pub const AGENA_MARKETPLACE_FILENAME: &str = "agena-marketplace.json";
pub const AGENA_RELEASE_MANIFEST_FILENAME: &str = "agena-plugin-release.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
/// Human-facing marketplace identity. It is metadata only; plugin installation
/// authority still comes from immutable release manifests and verified assets.
pub struct MarketplaceMetadata {
    pub name: String,
    pub description: String,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub owner: Option<MarketplaceOwner>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MarketplaceOwner {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Deterministic marketplace registry. Stable plugin ids are never silently
/// reused: explicit rename chains preserve old install locators, following the
/// same immutable-slug principle used by mature agent plugin marketplaces.
pub struct RegistryIndex {
    #[serde(default = "default_index_version")]
    pub version: u32,
    pub marketplace: MarketplaceMetadata,
    pub renames: BTreeMap<String, String>,
    pub plugins: Vec<PluginRecord>,
}

fn normalize_github_repository_url(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = value.strip_prefix("https://github.com/")?;
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("https://github.com/{owner}/{repository}"))
}

impl Default for RegistryIndex {
    fn default() -> Self {
        Self {
            version: default_index_version(),
            marketplace: MarketplaceMetadata::default(),
            renames: BTreeMap::new(),
            plugins: Vec::new(),
        }
    }
}

impl PluginVersion {
    fn validate(&self, plugin_id: &str) -> Result<(), MarketplaceError> {
        semver::Version::parse(self.version.trim_start_matches('v')).map_err(|error| {
            MarketplaceError::Index(format!(
                "plugin `{plugin_id}` has invalid version `{}`: {error}",
                self.version
            ))
        })?;
        if self.platform.trim().is_empty() {
            return Err(MarketplaceError::Index(format!(
                "plugin `{plugin_id}` has an empty artifact platform"
            )));
        }
        if self.url.trim().is_empty() {
            return Err(MarketplaceError::Index(format!(
                "plugin `{plugin_id}` has an empty artifact URL"
            )));
        }
        if let Some(sha256) = self.sha256.as_deref()
            && (sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(MarketplaceError::Index(format!(
                "plugin `{plugin_id}` has an invalid sha256 for {}",
                self.platform
            )));
        }
        if let Some(ArchiveSpec::TarGz { entrypoint }) = self.archive.as_ref() {
            validate_relative_asset_path(entrypoint, "archive entrypoint")?;
        }
        if let Some(source) = self.source.as_ref() {
            source.validate(plugin_id)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Immutable source provenance captured by GitHub Actions when a release is
/// assembled. The commit is the code identity; tag and workflow URL make the
/// release auditable without trusting mutable branches.
pub struct PluginReleaseSource {
    pub repository: String,
    pub tag: String,
    pub commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_run_url: Option<String>,
}

impl PluginReleaseSource {
    fn validate(&self, plugin_id: &str) -> Result<(), MarketplaceError> {
        let repository = normalize_github_repository_url(&self.repository).ok_or_else(|| {
            MarketplaceError::Index(format!(
                "plugin `{plugin_id}` release source repository `{}` is not a canonical GitHub URL",
                self.repository
            ))
        })?;
        if self.tag.trim().is_empty() || self.tag.contains('/') {
            return Err(MarketplaceError::Index(format!(
                "plugin `{plugin_id}` release source tag `{}` is invalid",
                self.tag
            )));
        }
        if self.commit.len() != 40 || !self.commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(MarketplaceError::Index(format!(
                "plugin `{plugin_id}` release source commit must be a 40-character Git SHA"
            )));
        }
        if let Some(workflow_url) = self.workflow_run_url.as_deref() {
            let prefix = format!("{repository}/actions/runs/");
            let run_id = workflow_url.strip_prefix(&prefix).ok_or_else(|| {
                MarketplaceError::Index(format!(
                    "plugin `{plugin_id}` workflow URL `{workflow_url}` does not belong to `{repository}`"
                ))
            })?;
            if run_id.is_empty() || !run_id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(MarketplaceError::Index(format!(
                    "plugin `{plugin_id}` workflow URL `{workflow_url}` has an invalid run id"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// Immutable release manifest uploaded alongside GitHub Release assets.
/// One manifest can describe every platform build for a single plugin version.
pub struct PluginReleaseManifest {
    #[serde(default = "default_index_version")]
    pub schema_version: u32,
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_agena_version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PluginReleaseSource>,
    pub artifacts: Vec<PluginReleaseArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginReleaseArtifact {
    pub target: String,
    pub kind: PluginKind,
    /// Release asset filename. It must be a relative safe path and is resolved
    /// next to the release manifest when `url` is absent.
    pub asset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<PluginSignature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub settings: serde_json::Value,
}

impl PluginReleaseManifest {
    pub fn validate(&self) -> Result<(), MarketplaceError> {
        if self.schema_version != 1 {
            return Err(MarketplaceError::Index(format!(
                "unsupported release schema version {}",
                self.schema_version
            )));
        }
        self.id
            .parse::<agena_plugin_host::PluginKey>()
            .map_err(|error| MarketplaceError::Index(error.to_string()))?;
        semver::Version::parse(self.version.trim_start_matches('v')).map_err(|error| {
            MarketplaceError::Index(format!(
                "plugin `{}` has invalid release version `{}`: {error}",
                self.id, self.version
            ))
        })?;
        if let Some(minimum) = self.min_agena_version.as_deref() {
            semver::VersionReq::parse(minimum).map_err(|error| {
                MarketplaceError::Index(format!(
                    "plugin `{}` has invalid min_agena_version `{minimum}`: {error}",
                    self.id
                ))
            })?;
        }
        if let Some(source) = self.source.as_ref() {
            source.validate(self.id.as_str())?;
        }
        if self.artifacts.is_empty() {
            return Err(MarketplaceError::Index(format!(
                "plugin `{}` release has no artifacts",
                self.id
            )));
        }
        let mut identities = std::collections::BTreeSet::new();
        for artifact in &self.artifacts {
            validate_relative_asset_path(artifact.asset.as_str(), "release asset")?;
            let version = PluginVersion {
                version: self.version.clone(),
                kind: artifact.kind,
                platform: artifact.target.clone(),
                url: artifact
                    .url
                    .clone()
                    .unwrap_or_else(|| artifact.asset.clone()),
                sha256: Some(artifact.sha256.clone()),
                signature: artifact.signature.clone(),
                command: artifact.command.clone(),
                args: artifact.args.clone(),
                env: artifact.env.clone(),
                settings: artifact.settings.clone(),
                min_agena_version: self.min_agena_version.clone(),
                archive: artifact.archive.clone(),
                dependencies: self.dependencies.clone(),
                source: self.source.clone(),
            };
            version.validate(self.id.as_str())?;
            if !identities.insert((artifact.target.as_str(), artifact.kind.as_ref())) {
                return Err(MarketplaceError::Index(format!(
                    "plugin `{}` release repeats target/kind {} / {}",
                    self.id, artifact.target, artifact.kind
                )));
            }
        }
        Ok(())
    }

    /// Additional policy used by GitHub-backed public marketplaces. The
    /// general release schema remains mirror-friendly, while official catalogs
    /// can require immutable assets from the declared repository and version.
    pub fn validate_github_distribution(&self) -> Result<(), MarketplaceError> {
        self.validate()?;
        let repository = self.repository.as_deref().ok_or_else(|| {
            MarketplaceError::Index(format!(
                "plugin `{}` must declare a GitHub repository",
                self.id
            ))
        })?;
        let repository = normalize_github_repository_url(repository).ok_or_else(|| {
            MarketplaceError::Index(format!(
                "plugin `{}` repository `{repository}` is not a canonical GitHub repository URL",
                self.id
            ))
        })?;
        let source = self.source.as_ref().ok_or_else(|| {
            MarketplaceError::Index(format!(
                "plugin `{}` GitHub release must declare immutable source provenance",
                self.id
            ))
        })?;
        source.validate(self.id.as_str())?;
        if normalize_github_repository_url(&source.repository).as_deref()
            != Some(repository.as_str())
        {
            return Err(MarketplaceError::Index(format!(
                "plugin `{}` release source repository does not match its declared repository",
                self.id
            )));
        }
        let version = self.version.trim_start_matches('v');
        if source.tag.trim_start_matches('v') != version {
            return Err(MarketplaceError::Index(format!(
                "plugin `{}` source tag `{}` does not match release version `{}`",
                self.id, source.tag, self.version
            )));
        }
        for artifact in &self.artifacts {
            let url = artifact.url.as_deref().ok_or_else(|| {
                MarketplaceError::Index(format!(
                    "plugin `{}` artifact `{}` must declare an immutable GitHub Release URL",
                    self.id, artifact.asset
                ))
            })?;
            let expected_prefix = format!("{repository}/releases/download/");
            let remainder = url.strip_prefix(expected_prefix.as_str()).ok_or_else(|| {
                MarketplaceError::Index(format!(
                    "plugin `{}` artifact URL `{url}` is not served by its declared GitHub repository",
                    self.id
                ))
            })?;
            let (tag, asset) = remainder.split_once('/').ok_or_else(|| {
                MarketplaceError::Index(format!(
                    "plugin `{}` artifact URL `{url}` is not an immutable GitHub Release asset",
                    self.id
                ))
            })?;
            if tag != source.tag
                || tag.trim_start_matches('v') != version
                || asset != artifact.asset
            {
                return Err(MarketplaceError::Index(format!(
                    "plugin `{}` artifact URL `{url}` does not match version `{}` and asset `{}`",
                    self.id, self.version, artifact.asset
                )));
            }
        }
        Ok(())
    }

    pub fn to_registry_index(
        &self,
        manifest_source: Option<&str>,
    ) -> Result<RegistryIndex, MarketplaceError> {
        Ok(RegistryIndex {
            version: 1,
            marketplace: MarketplaceMetadata::default(),
            renames: BTreeMap::new(),
            plugins: vec![self.to_plugin_record(manifest_source)?],
        })
    }

    pub fn to_plugin_record(
        &self,
        manifest_source: Option<&str>,
    ) -> Result<PluginRecord, MarketplaceError> {
        self.validate()?;
        let mut versions = Vec::with_capacity(self.artifacts.len());
        for artifact in &self.artifacts {
            let url = match artifact.url.as_deref() {
                Some(url) if is_absolute_source(url) => url.to_string(),
                Some(url) => resolve_relative_source(manifest_source, url)?,
                None => resolve_relative_source(manifest_source, artifact.asset.as_str())?,
            };
            versions.push(PluginVersion {
                version: self.version.clone(),
                kind: artifact.kind,
                platform: artifact.target.clone(),
                url,
                sha256: Some(artifact.sha256.clone()),
                signature: artifact.signature.clone(),
                command: artifact.command.clone(),
                args: artifact.args.clone(),
                env: artifact.env.clone(),
                settings: artifact.settings.clone(),
                min_agena_version: self.min_agena_version.clone(),
                archive: artifact.archive.clone(),
                dependencies: self.dependencies.clone(),
                source: self.source.clone(),
            });
        }
        sort_versions(&mut versions);
        Ok(PluginRecord {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            homepage: self.homepage.clone(),
            repository: self.repository.clone(),
            license: self.license.clone(),
            category: self.category.clone(),
            tags: self.tags.clone(),
            review_tier: MarketplaceReviewTier::Community,
            featured: false,
            versions,
        })
    }
}

pub fn parse_registry_document(
    bytes: &[u8],
    source: &str,
) -> Result<RegistryIndex, MarketplaceError> {
    parse_registry_document_with_policy(bytes, source, false)
}

pub fn parse_registry_document_with_policy(
    bytes: &[u8],
    source: &str,
    require_github_distribution: bool,
) -> Result<RegistryIndex, MarketplaceError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("plugins").is_some() {
        let index: RegistryIndex = serde_json::from_value(value)?;
        if require_github_distribution {
            index.validate_github_distribution()?;
        } else {
            index.validate()?;
        }
        return Ok(index);
    }
    if value.get("artifacts").is_some() && value.get("id").is_some() {
        let release: PluginReleaseManifest = serde_json::from_value(value)?;
        if require_github_distribution {
            release.validate_github_distribution()?;
        }
        let index = release.to_registry_index(Some(source))?;
        if require_github_distribution {
            index.validate_github_distribution()?;
        } else {
            index.validate()?;
        }
        return Ok(index);
    }
    Err(MarketplaceError::Index(
        "expected an Agena marketplace index or plugin release manifest".to_string(),
    ))
}

fn is_absolute_source(value: &str) -> bool {
    value.starts_with("https://") || value.starts_with("http://") || value.starts_with("file://")
}

fn resolve_relative_source(base: Option<&str>, relative: &str) -> Result<String, MarketplaceError> {
    if is_absolute_source(relative) {
        return Ok(relative.to_string());
    }
    let base = base.ok_or_else(|| {
        MarketplaceError::Index(format!(
            "release asset `{relative}` is relative but no manifest source was supplied"
        ))
    })?;
    if let Some(path) = base.strip_prefix("file://") {
        let parent = std::path::Path::new(path)
            .parent()
            .ok_or_else(|| MarketplaceError::InvalidUrl(base.to_string()))?;
        return Ok(format!("file://{}", parent.join(relative).display()));
    }
    let base =
        reqwest::Url::parse(base).map_err(|_| MarketplaceError::InvalidUrl(base.to_string()))?;
    base.join(relative)
        .map(|url| url.to_string())
        .map_err(|_| MarketplaceError::InvalidUrl(relative.to_string()))
}

fn validate_relative_asset_path(value: &str, label: &str) -> Result<(), MarketplaceError> {
    let path = std::path::Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(MarketplaceError::Index(format!(
            "{label} `{value}` must be a safe relative path"
        )));
    }
    Ok(())
}

fn sort_versions(versions: &mut [PluginVersion]) {
    versions.sort_by(|left, right| {
        let left_version = semver::Version::parse(left.version.trim_start_matches('v')).ok();
        let right_version = semver::Version::parse(right.version.trim_start_matches('v')).ok();
        right_version
            .cmp(&left_version)
            .then_with(|| left.platform.cmp(&right.platform))
            .then_with(|| left.kind.as_ref().cmp(right.kind.as_ref()))
    });
}

impl PluginRecord {
    pub fn validate(&self) -> Result<(), MarketplaceError> {
        self.id
            .parse::<agena_plugin_host::PluginKey>()
            .map_err(|error| MarketplaceError::Index(error.to_string()))?;
        if self.versions.is_empty() {
            return Err(MarketplaceError::Index(format!(
                "plugin `{}` has no versions",
                self.id
            )));
        }
        let mut identities = std::collections::BTreeSet::new();
        for version in &self.versions {
            version.validate(self.id.as_str())?;
            let identity = (
                version.version.as_str(),
                version.platform.as_str(),
                version.kind.as_ref(),
            );
            if !identities.insert(identity) {
                return Err(MarketplaceError::Index(format!(
                    "plugin `{}` repeats version/platform/kind {} / {} / {}",
                    self.id, version.version, version.platform, version.kind
                )));
            }
        }
        Ok(())
    }
}

impl RegistryIndex {
    pub fn validate(&self) -> Result<(), MarketplaceError> {
        if self.version != 1 {
            return Err(MarketplaceError::Index(format!(
                "unsupported marketplace schema version {}",
                self.version
            )));
        }
        let mut plugin_ids = std::collections::BTreeSet::new();
        for plugin in &self.plugins {
            plugin.validate()?;
            if !plugin_ids.insert(plugin.id.as_str()) {
                return Err(MarketplaceError::Index(format!(
                    "duplicate plugin id `{}`",
                    plugin.id
                )));
            }
        }
        for (alias, target) in &self.renames {
            alias
                .parse::<agena_plugin_host::PluginKey>()
                .map_err(|error| {
                    MarketplaceError::Index(format!(
                        "invalid marketplace rename source `{alias}`: {error}"
                    ))
                })?;
            target
                .parse::<agena_plugin_host::PluginKey>()
                .map_err(|error| {
                    MarketplaceError::Index(format!(
                        "invalid marketplace rename target `{target}`: {error}"
                    ))
                })?;
            if alias == target {
                return Err(MarketplaceError::Index(format!(
                    "marketplace rename `{alias}` cannot point to itself"
                )));
            }
            if plugin_ids.contains(alias.as_str()) {
                return Err(MarketplaceError::Index(format!(
                    "marketplace rename source `{alias}` is still an active plugin id"
                )));
            }
            self.resolve_plugin_id(alias)?;
        }
        if let Some(repository) = self.marketplace.repository.as_deref()
            && normalize_github_repository_url(repository).is_none()
        {
            return Err(MarketplaceError::Index(format!(
                "marketplace repository `{repository}` is not a canonical GitHub repository URL"
            )));
        }
        Ok(())
    }

    /// Public GitHub-backed registries are stricter than mirror/local indexes:
    /// every version must point to an immutable GitHub Release asset and carry
    /// exact source provenance for the same repository/tag/version.
    pub fn validate_github_distribution(&self) -> Result<(), MarketplaceError> {
        self.validate()?;
        for plugin in &self.plugins {
            let repository = plugin.repository.as_deref().ok_or_else(|| {
                MarketplaceError::Index(format!(
                    "plugin `{}` must declare a GitHub repository",
                    plugin.id
                ))
            })?;
            let repository = normalize_github_repository_url(repository).ok_or_else(|| {
                MarketplaceError::Index(format!(
                    "plugin `{}` repository `{repository}` is not a canonical GitHub repository URL",
                    plugin.id
                ))
            })?;
            for version in &plugin.versions {
                let source = version.source.as_ref().ok_or_else(|| {
                    MarketplaceError::Index(format!(
                        "plugin `{}` version `{}` must declare immutable GitHub source provenance",
                        plugin.id, version.version
                    ))
                })?;
                source.validate(plugin.id.as_str())?;
                if normalize_github_repository_url(&source.repository).as_deref()
                    != Some(repository.as_str())
                {
                    return Err(MarketplaceError::Index(format!(
                        "plugin `{}` version `{}` source repository does not match its declared repository",
                        plugin.id, version.version
                    )));
                }
                if source.tag.trim_start_matches('v') != version.version.trim_start_matches('v') {
                    return Err(MarketplaceError::Index(format!(
                        "plugin `{}` source tag `{}` does not match version `{}`",
                        plugin.id, source.tag, version.version
                    )));
                }
                let expected_prefix = format!("{repository}/releases/download/{}/", source.tag);
                let asset = version.url.strip_prefix(expected_prefix.as_str()).ok_or_else(|| {
                    MarketplaceError::Index(format!(
                        "plugin `{}` version `{}` URL `{}` is not an immutable GitHub Release asset",
                        plugin.id, version.version, version.url
                    ))
                })?;
                if asset.is_empty() || asset.contains('/') {
                    return Err(MarketplaceError::Index(format!(
                        "plugin `{}` version `{}` URL `{}` has an invalid GitHub Release asset name",
                        plugin.id, version.version, version.url
                    )));
                }
            }
        }
        Ok(())
    }

    /// Resolve an immutable old slug through the explicit rename graph. Rename
    /// chains are allowed for long-lived catalogs, but cycles and dangling
    /// targets are rejected by the same method used by validation and install.
    pub fn resolve_plugin_id(&self, requested: &str) -> Result<String, MarketplaceError> {
        requested
            .parse::<agena_plugin_host::PluginKey>()
            .map_err(|error| MarketplaceError::Index(error.to_string()))?;
        let mut current = requested.to_string();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(next) = self.renames.get(&current) {
            if !seen.insert(current.clone()) {
                return Err(MarketplaceError::Index(format!(
                    "marketplace rename cycle detected at `{current}`"
                )));
            }
            current = next.clone();
        }
        if !self.plugins.iter().any(|plugin| plugin.id == current) {
            return Err(MarketplaceError::PluginNotFound(current));
        }
        Ok(current)
    }

    pub fn upsert_release(
        &mut self,
        release: &PluginReleaseManifest,
    ) -> Result<(), MarketplaceError> {
        release.validate()?;
        let record = release.to_plugin_record(None)?;
        if let Some(existing) = self
            .plugins
            .iter_mut()
            .find(|plugin| plugin.id == record.id)
        {
            existing.name = record.name;
            existing.description = record.description;
            existing.homepage = record.homepage;
            existing.repository = record.repository;
            existing.license = record.license;
            existing.category = record.category;
            existing.tags = record.tags;
            let release_version = release.version.as_str();
            existing
                .versions
                .retain(|version| version.version != release_version);
            existing.versions.extend(record.versions);
            sort_versions(&mut existing.versions);
        } else {
            self.plugins.push(record);
            self.plugins.sort_by(|left, right| left.id.cmp(&right.id));
        }
        self.validate()
    }
}

fn default_index_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A plugin entry in the registry index.
pub struct PluginRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Human marketplace-review classification. This is deliberately separate
    /// from source provenance: a commit can be cryptographically attributable
    /// without being officially maintained or marketplace-reviewed.
    #[serde(default)]
    pub review_tier: MarketplaceReviewTier,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub featured: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<PluginVersion>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MarketplaceReviewTier {
    Official,
    Verified,
    #[default]
    Community,
}

impl MarketplaceReviewTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Verified => "verified",
            Self::Community => "community",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A version of a marketplace plugin.
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
    pub settings: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_agena_version: Option<String>,
    /// When set, the artifact bytes are treated as an archive and extracted
    /// under `plugins/<id>/<version>/` instead of being written verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveSpec>,
    /// Optional dependency list resolved at install time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencySpec>,
    /// Immutable GitHub source provenance when the version came from a public
    /// release. Mirrors and local registries may omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PluginReleaseSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "format", rename_all = "snake_case")]
/// How a plugin archive is fetched.
pub enum ArchiveSpec {
    /// gzip tar archive. The named entrypoint inside the archive is what the
    /// final config's `command`/`path` field will point to.
    TarGz { entrypoint: String },
}

/// A single dependency reference: another plugin id + a semver requirement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencySpec {
    pub plugin_id: String,
    pub version_req: String,
}

fn default_platform() -> String {
    "any".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a marketplace plugin.
pub enum PluginKind {
    Cdylib,
    Stdio,
    Http,
    Wasm,
}

impl AsRef<str> for PluginKind {
    fn as_ref(&self) -> &str {
        match self {
            Self::Cdylib => "cdylib",
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Wasm => "wasm",
        }
    }
}

impl fmt::Display for PluginKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl PluginKind {
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
