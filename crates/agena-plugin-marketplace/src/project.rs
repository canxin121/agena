//! Developer-facing plugin project, packaging, release assembly, and scaffold
//! primitives. These are deliberately language-neutral at the manifest layer;
//! the first bundled scaffolds target Rust stdio and cdylib plugins.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::MarketplaceError;
use crate::manifest::{
    AGENA_RELEASE_MANIFEST_FILENAME, ArchiveSpec, DependencySpec, MarketplaceMetadata,
    MarketplaceOwner, MarketplaceReviewTier, PluginKind, PluginReleaseArtifact,
    PluginReleaseManifest, PluginReleaseSource, RegistryIndex,
};

pub const AGENA_PROJECT_MANIFEST_FILENAME: &str = "agena-plugin.toml";
pub const AGENA_MARKETPLACE_PROJECT_FILENAME: &str = "agena-marketplace.toml";
/// Agena revision used by newly scaffolded standalone repositories. During
/// development this follows the current tree; before publishing templates we
/// replace it with the verified ecosystem commit SHA so generated repositories
/// never depend on a mutable branch.
pub const AGENA_TEMPLATE_BASELINE_REF: &str = "151e6f388d41048a989d8694a01238dbe4349722";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginProjectManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub plugin: PluginProjectMetadata,
    #[serde(default)]
    pub release: PluginProjectRelease,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MarketplaceProjectManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub marketplace: MarketplaceMetadata,
    #[serde(default)]
    pub renames: BTreeMap<String, String>,
    #[serde(default)]
    pub plugins: BTreeMap<String, MarketplacePluginPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct MarketplacePluginPolicy {
    pub review_tier: MarketplaceReviewTier,
    pub featured: bool,
}

impl MarketplaceProjectManifest {
    pub fn load(path: &Path) -> Result<Self, MarketplaceError> {
        let manifest: Self = toml::from_str(&fs::read_to_string(path)?)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), MarketplaceError> {
        if self.schema_version != 1 {
            return Err(MarketplaceError::Project(format!(
                "unsupported marketplace project schema version {}",
                self.schema_version
            )));
        }
        if self.marketplace.name.trim().is_empty() || self.marketplace.description.trim().is_empty()
        {
            return Err(MarketplaceError::Project(
                "marketplace name and description must not be empty".to_string(),
            ));
        }
        if let Some(repository) = self.marketplace.repository.as_deref() {
            validate_canonical_github_repository(repository)?;
        }
        for (alias, target) in &self.renames {
            agena_plugin_contracts::validate_plugin_identity(alias)
                .map_err(|error| MarketplaceError::Project(error.to_string()))?;
            agena_plugin_contracts::validate_plugin_identity(target)
                .map_err(|error| MarketplaceError::Project(error.to_string()))?;
            if alias == target {
                return Err(MarketplaceError::Project(format!(
                    "marketplace rename `{alias}` cannot point to itself"
                )));
            }
        }
        for plugin_id in self.plugins.keys() {
            agena_plugin_contracts::validate_plugin_identity(plugin_id)
                .map_err(|error| MarketplaceError::Project(error.to_string()))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ScaffoldMarketplaceRequest {
    pub destination: PathBuf,
    pub name: String,
    pub description: String,
    pub repository: String,
    pub owner_name: String,
    pub force: bool,
}

/// Scaffold a GitHub-first marketplace repository. Release manifests are the
/// reviewed source files; `agena-marketplace.json` is deterministic generated
/// output checked in CI.
pub fn scaffold_marketplace(request: ScaffoldMarketplaceRequest) -> Result<(), MarketplaceError> {
    validate_canonical_github_repository(request.repository.as_str())?;
    if request.name.trim().is_empty()
        || request.description.trim().is_empty()
        || request.owner_name.trim().is_empty()
    {
        return Err(MarketplaceError::Project(
            "marketplace name, description, and owner must not be empty".to_string(),
        ));
    }
    if request.destination.exists()
        && request.destination.read_dir()?.next().is_some()
        && !request.force
    {
        return Err(MarketplaceError::Project(format!(
            "destination {} is not empty; pass --force to overwrite template files",
            request.destination.display()
        )));
    }
    fs::create_dir_all(request.destination.join("releases"))?;
    fs::create_dir_all(request.destination.join(".github/workflows"))?;
    fs::create_dir_all(request.destination.join(".github/ISSUE_TEMPLATE"))?;
    let project = MarketplaceProjectManifest {
        schema_version: 1,
        marketplace: MarketplaceMetadata {
            name: request.name.clone(),
            description: request.description.clone(),
            homepage: Some(request.repository.clone()),
            repository: Some(request.repository.clone()),
            owner: Some(MarketplaceOwner {
                name: request.owner_name.clone(),
                url: None,
            }),
        },
        renames: BTreeMap::new(),
        plugins: BTreeMap::new(),
    };
    write_text(
        &request.destination.join(AGENA_MARKETPLACE_PROJECT_FILENAME),
        toml::to_string_pretty(&project)?.as_str(),
    )?;
    write_text(&request.destination.join("releases/.gitkeep"), "")?;
    build_marketplace(BuildMarketplaceRequest {
        releases_dir: request.destination.join("releases"),
        project_path: Some(request.destination.join(AGENA_MARKETPLACE_PROJECT_FILENAME)),
        output_path: request
            .destination
            .join(crate::manifest::AGENA_MARKETPLACE_FILENAME),
        github_only: false,
    })?;
    write_text(
        &request.destination.join("README.md"),
        marketplace_readme_template(&request).as_str(),
    )?;
    write_text(
        &request.destination.join("CONTRIBUTING.md"),
        MARKETPLACE_CONTRIBUTING,
    )?;
    write_text(
        &request.destination.join("SECURITY.md"),
        MARKETPLACE_SECURITY,
    )?;
    write_text(
        &request.destination.join("LICENSE"),
        apache_license(&request.owner_name).as_str(),
    )?;
    write_text(
        &request.destination.join(".github/workflows/validate.yml"),
        marketplace_validate_workflow().as_str(),
    )?;
    write_text(
        &request.destination.join(".github/pull_request_template.md"),
        MARKETPLACE_PULL_REQUEST_TEMPLATE,
    )?;
    if let Some(owner) = github_repository_owner(request.repository.as_str()) {
        write_text(
            &request.destination.join(".github/CODEOWNERS"),
            format!("* @{owner}\n").as_str(),
        )?;
    }
    Ok(())
}

fn github_repository_owner(value: &str) -> Option<&str> {
    value
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .strip_prefix("https://github.com/")?
        .split('/')
        .next()
        .filter(|owner| !owner.is_empty())
}

/// Generate the standalone repository lockfile after scaffolding. The template
/// CI and release workflows always use `--locked`, so a newly-created plugin
/// repository must commit this file to make the first CI run reproducible.
pub fn generate_plugin_lockfile(project_root: &Path) -> Result<(), MarketplaceError> {
    let status = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(project_root)
        .status()
        .map_err(|error| {
            MarketplaceError::Project(format!(
                "failed to run cargo generate-lockfile in {}: {error}",
                project_root.display()
            ))
        })?;
    if !status.success() {
        return Err(MarketplaceError::Project(format!(
            "cargo generate-lockfile failed in {} with status {status}",
            project_root.display()
        )));
    }
    let lockfile = project_root.join("Cargo.lock");
    if !lockfile.is_file() {
        return Err(MarketplaceError::Project(format!(
            "cargo generate-lockfile succeeded but {} was not created",
            lockfile.display()
        )));
    }
    Ok(())
}

fn validate_canonical_github_repository(value: &str) -> Result<(), MarketplaceError> {
    let value = value.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = value.strip_prefix("https://github.com/").ok_or_else(|| {
        MarketplaceError::Project(format!(
            "repository `{value}` must be a canonical https://github.com/OWNER/REPO URL"
        ))
    })?;
    let mut parts = path.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        return Err(MarketplaceError::Project(format!(
            "repository `{value}` must be a canonical https://github.com/OWNER/REPO URL"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AddMarketplaceReleaseRequest {
    pub release_path: PathBuf,
    pub releases_dir: PathBuf,
    pub project_path: Option<PathBuf>,
    pub index_path: PathBuf,
    pub github_only: bool,
}

#[derive(Debug, Clone)]
pub struct AddMarketplaceReleaseOutcome {
    pub stored_release_path: PathBuf,
    pub index_path: PathBuf,
    pub plugin_count: usize,
    pub release_count: usize,
    pub already_present: bool,
}

/// Store one immutable release manifest in the marketplace source tree and
/// rebuild the derived index. Published plugin-id/version pairs are immutable:
/// resubmitting identical content is a no-op, while changing an existing
/// version is rejected and requires a new plugin version.
pub fn add_marketplace_release(
    request: AddMarketplaceReleaseRequest,
) -> Result<AddMarketplaceReleaseOutcome, MarketplaceError> {
    let release: PluginReleaseManifest = serde_json::from_slice(&fs::read(&request.release_path)?)?;
    if request.github_only {
        release.validate_github_distribution()?;
    } else {
        release.validate()?;
    }
    let version = release.version.trim_start_matches('v');
    let stored_release_path = request
        .releases_dir
        .join(release.id.as_str())
        .join(format!("{version}.json"));
    let already_present = if stored_release_path.exists() {
        let existing: PluginReleaseManifest =
            serde_json::from_slice(&fs::read(&stored_release_path)?)?;
        if existing != release {
            return Err(MarketplaceError::Project(format!(
                "marketplace release {}@{} is immutable; publish a new version instead of replacing {}",
                release.id,
                release.version,
                stored_release_path.display()
            )));
        }
        true
    } else {
        write_json(&stored_release_path, &release)?;
        false
    };
    let built = build_marketplace(BuildMarketplaceRequest {
        releases_dir: request.releases_dir,
        project_path: request.project_path,
        output_path: request.index_path.clone(),
        github_only: request.github_only,
    })?;
    Ok(AddMarketplaceReleaseOutcome {
        stored_release_path,
        index_path: built.output_path,
        plugin_count: built.plugin_count,
        release_count: built.release_count,
        already_present,
    })
}

#[derive(Debug, Clone)]
pub struct BuildMarketplaceRequest {
    pub releases_dir: PathBuf,
    pub project_path: Option<PathBuf>,
    pub output_path: PathBuf,
    pub github_only: bool,
}

#[derive(Debug, Clone)]
pub struct BuildMarketplaceOutcome {
    pub output_path: PathBuf,
    pub plugin_count: usize,
    pub release_count: usize,
}

/// Deterministically rebuild a marketplace index from one immutable release
/// manifest per plugin version. This keeps pull requests conflict-light and
/// makes the generated catalog fully reproducible in CI.
pub fn build_marketplace(
    request: BuildMarketplaceRequest,
) -> Result<BuildMarketplaceOutcome, MarketplaceError> {
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(&request.releases_dir)
        .follow_links(false)
        .into_iter()
    {
        let entry = entry.map_err(|error| MarketplaceError::Project(error.to_string()))?;
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            paths.push(entry.into_path());
        }
    }

    paths.sort();
    let project_path = request.project_path.or_else(|| {
        request
            .releases_dir
            .parent()
            .map(|parent| parent.join(AGENA_MARKETPLACE_PROJECT_FILENAME))
    });
    let project = if let Some(path) = project_path.filter(|path| path.is_file()) {
        MarketplaceProjectManifest::load(&path)?
    } else {
        MarketplaceProjectManifest {
            schema_version: 1,
            marketplace: MarketplaceMetadata::default(),
            renames: BTreeMap::new(),
            plugins: BTreeMap::new(),
        }
    };
    let mut index = RegistryIndex {
        marketplace: project.marketplace,
        renames: project.renames,
        ..RegistryIndex::default()
    };
    let plugin_policies = project.plugins;
    let mut release_ids = BTreeSet::new();
    for path in paths {
        let release: PluginReleaseManifest = serde_json::from_slice(&fs::read(&path)?)?;
        release.validate()?;
        if request.github_only {
            release.validate_github_distribution()?;
        }
        let identity = (release.id.clone(), release.version.clone());
        if !release_ids.insert(identity.clone()) {
            return Err(MarketplaceError::Project(format!(
                "duplicate release manifest {}@{}",
                identity.0, identity.1
            )));
        }
        index.upsert_release(&release)?;
    }
    for (plugin_id, policy) in plugin_policies {
        let record = index
            .plugins
            .iter_mut()
            .find(|plugin| plugin.id == plugin_id)
            .ok_or_else(|| {
                MarketplaceError::Project(format!(
                    "marketplace policy references unknown plugin `{plugin_id}`"
                ))
            })?;
        record.review_tier = policy.review_tier;
        record.featured = policy.featured;
    }
    if request.github_only {
        index.validate_github_distribution()?;
    } else {
        index.validate()?;
    }
    write_json(&request.output_path, &index)?;
    Ok(BuildMarketplaceOutcome {
        output_path: request.output_path,
        plugin_count: index.plugins.len(),
        release_count: release_ids.len(),
    })
}

fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginProjectMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub kind: PluginKind,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub min_agena_version: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub settings: serde_json::Value,
    #[serde(default)]
    pub dependencies: Vec<DependencySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PluginProjectRelease {
    pub entrypoint: Option<String>,
    pub include: Vec<String>,
}

impl PluginProjectManifest {
    pub fn load(path: &Path) -> Result<Self, MarketplaceError> {
        let text = fs::read_to_string(path)?;
        let mut manifest: Self = toml::from_str(&text)?;
        if manifest.plugin.version == "cargo" {
            manifest.plugin.version = cargo_package_version(
                &path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("Cargo.toml"),
            )?;
        }
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), MarketplaceError> {
        if self.schema_version != 1 {
            return Err(MarketplaceError::Project(format!(
                "unsupported project schema version {}",
                self.schema_version
            )));
        }
        agena_plugin_contracts::validate_plugin_identity(self.plugin.id.as_str())
            .map_err(|error| MarketplaceError::Project(error.to_string()))?;
        semver::Version::parse(self.plugin.version.trim_start_matches('v')).map_err(|error| {
            MarketplaceError::Project(format!(
                "invalid plugin version `{}`: {error}",
                self.plugin.version
            ))
        })?;
        if matches!(self.plugin.kind, PluginKind::Http) {
            return Err(MarketplaceError::Project(
                "HTTP plugins are remote endpoints and cannot be packaged as release artifacts"
                    .to_string(),
            ));
        }
        if let Some(minimum) = self.plugin.min_agena_version.as_deref() {
            semver::VersionReq::parse(minimum).map_err(|error| {
                MarketplaceError::Project(format!("invalid min_agena_version `{minimum}`: {error}"))
            })?;
        }
        if let Some(entrypoint) = self.release.entrypoint.as_deref() {
            validate_relative_path(entrypoint, "release.entrypoint")?;
        }
        for include in &self.release.include {
            validate_relative_path(include, "release.include")?;
        }
        let mut dependencies = BTreeSet::new();
        for dependency in &self.plugin.dependencies {
            agena_plugin_contracts::validate_plugin_identity(dependency.plugin_id.as_str())
                .map_err(|error| MarketplaceError::Project(error.to_string()))?;
            semver::VersionReq::parse(&dependency.version_req).map_err(|error| {
                MarketplaceError::Project(format!(
                    "invalid dependency requirement `{}` for `{}`: {error}",
                    dependency.version_req, dependency.plugin_id
                ))
            })?;
            if !dependencies.insert(dependency.plugin_id.as_str()) {
                return Err(MarketplaceError::Project(format!(
                    "duplicate dependency `{}`",
                    dependency.plugin_id
                )));
            }
        }
        Ok(())
    }

    fn release_manifest(&self, artifact: PluginReleaseArtifact) -> PluginReleaseManifest {
        PluginReleaseManifest {
            schema_version: 1,
            id: self.plugin.id.clone(),
            name: self.plugin.name.clone(),
            description: self.plugin.description.clone(),
            homepage: self.plugin.homepage.clone(),
            repository: self.plugin.repository.clone(),
            license: self.plugin.license.clone(),
            category: self.plugin.category.clone(),
            tags: self.plugin.tags.clone(),
            version: self.plugin.version.clone(),
            min_agena_version: self.plugin.min_agena_version.clone(),
            dependencies: self.plugin.dependencies.clone(),
            source: None,
            artifacts: vec![artifact],
        }
    }
}

#[derive(Debug, Clone)]
pub struct PackagePluginRequest {
    pub manifest_path: PathBuf,
    pub artifact_path: PathBuf,
    pub target: String,
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PackagePluginOutcome {
    pub plugin_id: String,
    pub version: String,
    pub target: String,
    pub archive_path: PathBuf,
    pub fragment_path: PathBuf,
    pub sha256: String,
}

pub fn package_plugin(
    request: PackagePluginRequest,
) -> Result<PackagePluginOutcome, MarketplaceError> {
    let manifest = PluginProjectManifest::load(&request.manifest_path)?;
    if request.target.trim().is_empty() {
        return Err(MarketplaceError::Project(
            "package target cannot be empty".to_string(),
        ));
    }
    if !request.artifact_path.is_file() {
        return Err(MarketplaceError::Project(format!(
            "plugin artifact does not exist: {}",
            request.artifact_path.display()
        )));
    }
    let project_root = request
        .manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(&request.output_dir)?;
    let entrypoint = manifest
        .release
        .entrypoint
        .clone()
        .or_else(|| {
            request
                .artifact_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .ok_or_else(|| MarketplaceError::Project("artifact has no filename".to_string()))?;
    validate_relative_path(entrypoint.as_str(), "release entrypoint")?;
    let slug = release_slug(manifest.plugin.id.as_str());
    let archive_name = format!(
        "{slug}-v{}-{}.tar.gz",
        manifest.plugin.version, request.target
    );
    let archive_path = request.output_dir.join(&archive_name);
    write_archive(
        project_root,
        &request.artifact_path,
        entrypoint.as_str(),
        &manifest.release.include,
        &archive_path,
        manifest.plugin.kind,
    )?;
    let sha256 = sha256_file(&archive_path)?;
    let fragment_name = format!(
        "agena-plugin-release.{}.json",
        sanitize_filename(request.target.as_str())
    );
    let fragment_path = request.output_dir.join(fragment_name);
    let release = manifest.release_manifest(PluginReleaseArtifact {
        target: request.target.clone(),
        kind: manifest.plugin.kind,
        asset: archive_name,
        url: None,
        sha256: sha256.clone(),
        signature: None,
        archive: Some(ArchiveSpec::TarGz { entrypoint }),
        command: None,
        args: manifest.plugin.args.clone(),
        env: manifest.plugin.env.clone(),
        settings: manifest.plugin.settings.clone(),
    });
    release.validate()?;
    write_json(&fragment_path, &release)?;
    Ok(PackagePluginOutcome {
        plugin_id: manifest.plugin.id,
        version: manifest.plugin.version,
        target: request.target,
        archive_path,
        fragment_path,
        sha256,
    })
}

#[derive(Debug, Clone)]
pub struct AssembleReleaseRequest {
    pub fragments_dir: PathBuf,
    pub output_dir: PathBuf,
    pub base_url: String,
    pub expected_version: Option<String>,
    /// Immutable GitHub Actions provenance. When present, release assembly
    /// enforces GitHub-only distribution invariants before writing the final
    /// manifest.
    pub source: Option<PluginReleaseSource>,
}

#[derive(Debug, Clone)]
pub struct AssembleReleaseOutcome {
    pub release_manifest_path: PathBuf,
    pub artifact_count: usize,
}

pub fn assemble_release(
    request: AssembleReleaseRequest,
) -> Result<AssembleReleaseOutcome, MarketplaceError> {
    let mut fragments = Vec::new();
    for entry in fs::read_dir(&request.fragments_dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name
            .strip_prefix("agena-plugin-release.")
            .and_then(|name| name.strip_suffix(".json"))
            .is_some_and(|target| !target.is_empty())
        {
            let release: PluginReleaseManifest = serde_json::from_slice(&fs::read(&path)?)?;
            release.validate()?;
            fragments.push((path, release));
        }
    }
    fragments.sort_by(|left, right| left.0.cmp(&right.0));
    let (_, first) = fragments.first().ok_or_else(|| {
        MarketplaceError::Project(format!(
            "no release fragments found in {}",
            request.fragments_dir.display()
        ))
    })?;
    let mut release = first.clone();
    release.artifacts.clear();
    if let Some(expected) = request.expected_version.as_deref()
        && expected.trim_start_matches('v') != release.version.trim_start_matches('v')
    {
        return Err(MarketplaceError::Project(format!(
            "release tag/version `{expected}` does not match plugin version `{}`",
            release.version
        )));
    }
    fs::create_dir_all(&request.output_dir)?;
    for (_, fragment) in fragments {
        ensure_same_release(&release, &fragment)?;
        for mut artifact in fragment.artifacts {
            let packaged = request.fragments_dir.join(&artifact.asset);
            if !packaged.is_file() {
                return Err(MarketplaceError::Project(format!(
                    "release artifact `{}` is missing from {}",
                    artifact.asset,
                    request.fragments_dir.display()
                )));
            }
            let actual_sha256 = sha256_file(&packaged)?;
            if !actual_sha256.eq_ignore_ascii_case(&artifact.sha256) {
                return Err(MarketplaceError::Sha256Mismatch {
                    plugin: fragment.id.clone(),
                    expected: artifact.sha256,
                    got: actual_sha256,
                });
            }
            let output_asset = request.output_dir.join(&artifact.asset);
            if output_asset != packaged {
                fs::copy(&packaged, &output_asset)?;
            }
            artifact.url = Some(join_base_url(
                request.base_url.as_str(),
                artifact.asset.as_str(),
            )?);
            release.artifacts.push(artifact);
        }
    }
    release.artifacts.sort_by(|left, right| {
        left.target
            .cmp(&right.target)
            .then_with(|| left.kind.as_ref().cmp(right.kind.as_ref()))
    });
    release.source = request.source;
    release.validate()?;
    if release.source.is_some() {
        release.validate_github_distribution()?;
    }
    let release_manifest_path = request.output_dir.join(AGENA_RELEASE_MANIFEST_FILENAME);
    write_json(&release_manifest_path, &release)?;

    Ok(AssembleReleaseOutcome {
        release_manifest_path,
        artifact_count: release.artifacts.len(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTemplateKind {
    Stdio,
    Cdylib,
}

#[derive(Debug, Clone)]
pub struct ScaffoldPluginRequest {
    pub destination: PathBuf,
    pub plugin_id: String,
    pub crate_name: String,
    pub display_name: String,
    pub description: String,
    pub author: String,
    pub repository: Option<String>,
    pub kind: PluginTemplateKind,
    pub force: bool,
}

pub fn scaffold_plugin(request: ScaffoldPluginRequest) -> Result<(), MarketplaceError> {
    agena_plugin_contracts::validate_plugin_identity(request.plugin_id.as_str())
        .map_err(|error| MarketplaceError::Project(error.to_string()))?;
    if let Some(repository) = request.repository.as_deref() {
        validate_canonical_github_repository(repository)?;
    }
    if request.destination.exists()
        && request.destination.read_dir()?.next().is_some()
        && !request.force
    {
        return Err(MarketplaceError::Project(format!(
            "destination {} is not empty; pass --force to overwrite template files",
            request.destination.display()
        )));
    }
    fs::create_dir_all(request.destination.join("src"))?;
    fs::create_dir_all(request.destination.join(".github/workflows"))?;
    fs::create_dir_all(request.destination.join(".github/ISSUE_TEMPLATE"))?;
    let (namespace, name) = request.plugin_id.split_once('.').ok_or_else(|| {
        MarketplaceError::Project("plugin id must contain namespace and name".to_string())
    })?;
    let export = match request.kind {
        PluginTemplateKind::Stdio => "stdio",
        PluginTemplateKind::Cdylib => "cdylib",
    };
    let cargo = cargo_template(&request, export);
    let source = rust_source_template(namespace, name, &request.description, export);
    let project = project_manifest_template(&request);
    write_text(&request.destination.join("Cargo.toml"), cargo.as_str())?;
    let source_name = if matches!(request.kind, PluginTemplateKind::Stdio) {
        "main.rs"
    } else {
        "lib.rs"
    };
    write_text(
        &request.destination.join("src").join(source_name),
        source.as_str(),
    )?;
    write_text(
        &request.destination.join(AGENA_PROJECT_MANIFEST_FILENAME),
        project.as_str(),
    )?;
    write_text(
        &request.destination.join("README.md"),
        readme_template(&request).as_str(),
    )?;
    write_text(
        &request.destination.join("LICENSE"),
        apache_license(&request.author).as_str(),
    )?;
    write_text(
        &request.destination.join(".gitignore"),
        "/target\n/dist\n.DS_Store\n",
    )?;
    write_text(
        &request.destination.join("rust-toolchain.toml"),
        RUST_TOOLCHAIN,
    )?;
    write_text(&request.destination.join("CONTRIBUTING.md"), CONTRIBUTING)?;
    write_text(&request.destination.join("SECURITY.md"), SECURITY)?;
    write_text(
        &request.destination.join(".github/pull_request_template.md"),
        PULL_REQUEST_TEMPLATE,
    )?;
    write_text(
        &request.destination.join(".github/ISSUE_TEMPLATE/bug.yml"),
        BUG_ISSUE_TEMPLATE,
    )?;
    if let Some(owner) = request
        .repository
        .as_deref()
        .and_then(github_repository_owner)
    {
        write_text(
            &request.destination.join(".github/CODEOWNERS"),
            format!("* @{owner}\n").as_str(),
        )?;
    }
    write_text(
        &request.destination.join(".github/workflows/ci.yml"),
        template_ci_workflow().as_str(),
    )?;
    write_text(
        &request.destination.join(".github/workflows/release.yml"),
        template_release_workflow(&request).as_str(),
    )?;
    Ok(())
}

fn cargo_package_version(path: &Path) -> Result<String, MarketplaceError> {
    let text = fs::read_to_string(path).map_err(|error| {
        MarketplaceError::Project(format!(
            "version = `cargo` requires {}: {error}",
            path.display()
        ))
    })?;
    let document: toml::Value = toml::from_str(&text)?;
    document
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            MarketplaceError::Project(format!(
                "{} must define package.version as a string",
                path.display()
            ))
        })
}

fn write_archive(
    project_root: &Path,
    artifact_path: &Path,
    entrypoint: &str,
    includes: &[String],
    archive_path: &Path,
    kind: PluginKind,
) -> Result<(), MarketplaceError> {
    let file = fs::File::create(archive_path)?;
    let encoder = GzEncoder::new(file, Compression::best());
    let mut archive = tar::Builder::new(encoder);
    let metadata = fs::metadata(artifact_path)?;
    let mut header = tar::Header::new_gnu();
    header.set_metadata(&metadata);
    header.set_mode(if matches!(kind, PluginKind::Stdio) {
        0o755
    } else {
        0o644
    });
    header.set_cksum();
    archive.append_data(&mut header, entrypoint, fs::File::open(artifact_path)?)?;
    for include in includes {
        let source = project_root.join(include);
        let canonical_root = project_root.canonicalize()?;
        let canonical_source = source.canonicalize().map_err(|error| {
            MarketplaceError::Project(format!("include `{include}` cannot be read: {error}"))
        })?;
        if !canonical_source.starts_with(&canonical_root) {
            return Err(MarketplaceError::Project(format!(
                "include `{include}` escapes the plugin project"
            )));
        }
        if canonical_source.is_dir() {
            archive.append_dir_all(include, canonical_source)?;
        } else {
            archive.append_path_with_name(canonical_source, include)?;
        }
    }
    archive.finish()?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, MarketplaceError> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn ensure_same_release(
    expected: &PluginReleaseManifest,
    actual: &PluginReleaseManifest,
) -> Result<(), MarketplaceError> {
    let mut expected = expected.clone();
    let mut actual = actual.clone();
    expected.artifacts.clear();
    actual.artifacts.clear();
    if expected != actual {
        return Err(MarketplaceError::Project(format!(
            "release fragment for `{}` does not match the other plugin metadata",
            actual.id
        )));
    }
    Ok(())
}

fn join_base_url(base: &str, asset: &str) -> Result<String, MarketplaceError> {
    let mut base = base.trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err(MarketplaceError::Project(
            "release base URL cannot be empty".to_string(),
        ));
    }
    base.push('/');
    let base =
        reqwest::Url::parse(&base).map_err(|_| MarketplaceError::InvalidUrl(base.clone()))?;
    base.join(asset)
        .map(|url| url.to_string())
        .map_err(|_| MarketplaceError::InvalidUrl(asset.to_string()))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), MarketplaceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

fn write_text(path: &Path, text: &str) -> Result<(), MarketplaceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    Ok(())
}

fn validate_relative_path(value: &str, label: &str) -> Result<(), MarketplaceError> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(MarketplaceError::Project(format!(
            "{label} `{value}` must be a safe relative path"
        )));
    }
    Ok(())
}

fn release_slug(plugin_id: &str) -> String {
    plugin_id.replace('.', "-")
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn cargo_template(request: &ScaffoldPluginRequest, export: &str) -> String {
    let target = if export == "stdio" {
        format!(
            "[[bin]]\nname = \"{}\"\npath = \"src/main.rs\"",
            request.crate_name
        )
    } else {
        "[lib]\ncrate-type = [\"cdylib\"]".to_string()
    };
    let extra = if export == "cdylib" {
        "abi_stable = \"0.11.3\"\n"
    } else {
        ""
    };
    let ignored = if export == "cdylib" {
        "\"abi_stable\", \"async-trait\", \"schemars\", \"serde\", \"serde_json\", \"tokio\""
    } else {
        "\"async-trait\", \"schemars\", \"serde\", \"serde_json\", \"tokio\""
    };
    let repository = request
        .repository
        .as_deref()
        .map(|repository| format!("repository = \"{repository}\"\n"))
        .unwrap_or_default();
    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2024"
rust-version = "1.97"
license = "Apache-2.0"
description = "{}"
{}

{}

[dependencies]
agena-plugin-sdk = {{ git = "https://github.com/canxin121/agena", rev = "{}", features = ["{}"] }}
{}async-trait = "0.1"
schemars = "1.2"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
tokio = {{ version = "1", features = ["rt-multi-thread", "macros", "io-std", "io-util"] }}

[package.metadata.cargo-machete]
ignored = [{}]
"#,
        request.crate_name,
        request.description,
        repository,
        target,
        AGENA_TEMPLATE_BASELINE_REF,
        export,
        extra,
        ignored
    )
}

fn rust_source_template(namespace: &str, name: &str, description: &str, export: &str) -> String {
    format!(
        r#"use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct HelloInput {{
    #[arg(trim, non_empty)]
    name: String,
}}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HelloOutput {{
    message: String,
}}

#[derive(Default)]
struct ExamplePlugin;

#[agena_plugin(
    namespace = "{namespace}",
    name = "{name}",
    version = env!("CARGO_PKG_VERSION"),
    summary = "{description}",
    export = {export}
)]
impl ExamplePlugin {{
    #[tool(
        name = "hello",
        summary = "Return a friendly greeting.",
        read_only,
        concurrency_safe
    )]
    async fn hello(&self, input: &HelloInput) -> Result<HelloOutput> {{
        Ok(HelloOutput {{
            message: format!("Hello, {{}}!", input.name),
        }})
    }}
}}
"#
    )
}

fn project_manifest_template(request: &ScaffoldPluginRequest) -> String {
    let kind = match request.kind {
        PluginTemplateKind::Stdio => "stdio",
        PluginTemplateKind::Cdylib => "cdylib",
    };
    let repository = request
        .repository
        .as_deref()
        .map(|repository| format!("repository = \"{repository}\"\n"))
        .unwrap_or_default();
    format!(
        r#"schema_version = 1

[plugin]
id = "{}"
name = "{}"
description = "{}"
version = "cargo"
kind = "{}"
{}
license = "Apache-2.0"
category = "development"
tags = ["example", "template"]
min_agena_version = ">=0.1.0"

[release]
include = ["README.md", "LICENSE"]
"#,
        request.plugin_id, request.display_name, request.description, kind, repository,
    )
}

fn readme_template(request: &ScaffoldPluginRequest) -> String {
    let install_source = request
        .repository
        .as_deref()
        .and_then(|repository| repository.strip_prefix("https://github.com/"))
        .unwrap_or("OWNER/REPOSITORY")
        .trim_end_matches('/');
    format!(
        r#"# {}

{}

## Develop

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
agena-plugin validate .
```

`agena-plugin.toml` is the release source of truth. Its `version = "cargo"`
reads the version from `Cargo.toml`, so release metadata cannot drift from the
compiled plugin.

The generated repository pins the Agena SDK and reusable workflows to one
verified Agena revision and commits `Cargo.lock`. When updating the Agena
baseline, update the SDK `rev`, both workflow `uses`/`agena_ref` values, and
regenerate `Cargo.lock` together.

## Release

Push a tag matching the Cargo version, such as `v0.1.0`. The generated GitHub
workflow calls Agena's reusable release workflow, builds every supported target,
packages immutable assets, verifies every SHA-256 digest, and publishes
`agena-plugin-release.json` beside the archives.

Install the latest release directly from GitHub:

```bash
agena plugin install {}
```

Or submit the release manifest to an Agena marketplace repository so users can
install it by plugin id.
"#,
        request.display_name, request.description, install_source
    )
}

fn apache_license(author: &str) -> String {
    format!(
        "Copyright 2026 {author}\n\nLicensed under the Apache License, Version 2.0 (the \"License\");\nyou may not use this file except in compliance with the License.\nYou may obtain a copy of the License at\n\n    http://www.apache.org/licenses/LICENSE-2.0\n\nUnless required by applicable law or agreed to in writing, software\ndistributed under the License is distributed on an \"AS IS\" BASIS,\nWITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.\nSee the License for the specific language governing permissions and\nlimitations under the License.\n"
    )
}

fn template_ci_workflow() -> String {
    format!(r#"name: CI

on:
  pull_request:
  push:
    branches: [main, master]

jobs:
  plugin_ci:
    uses: canxin121/agena/.github/workflows/plugin-ci.yml@{ref}
    with:
      agena_ref: "{ref}"
"#
    , ref = AGENA_TEMPLATE_BASELINE_REF)
}

fn template_release_workflow(request: &ScaffoldPluginRequest) -> String {
    let kind = match request.kind {
        PluginTemplateKind::Stdio => "stdio",
        PluginTemplateKind::Cdylib => "cdylib",
    };
    format!(
        r#"name: Release

on:
  push:
    tags: ['v*']

permissions:
  contents: write

jobs:
  plugin_release:
    uses: canxin121/agena/.github/workflows/plugin-release.yml@{ref}
    with:
      crate_name: "{}"
      plugin_kind: "{}"
      agena_ref: "{ref}"
    secrets: inherit
"#,
        request.crate_name,
        kind,
        ref = AGENA_TEMPLATE_BASELINE_REF,
    )
}

fn marketplace_readme_template(request: &ScaffoldMarketplaceRequest) -> String {
    let shorthand = request
        .repository
        .trim_start_matches("https://github.com/")
        .trim_end_matches('/');
    format!(
        r#"# {}

{}

This is a GitHub-first Agena plugin marketplace. Reviewed release manifests
under `releases/<plugin-id>/<version>.json` are the source of truth;
`agena-marketplace.json` is deterministic generated output.

## Use this marketplace

```bash
agena plugin sync --registry {shorthand}
agena plugin search --registry {shorthand} QUERY
agena plugin install PLUGIN.ID --registry {shorthand}
```

## Add a published plugin release

Publish the plugin from its own GitHub repository first. Download that
release's `agena-plugin-release.json`, then run:

```bash
agena-plugin marketplace add /path/to/agena-plugin-release.json \
  --releases releases \
  --project agena-marketplace.toml \
  --index agena-marketplace.json \
  --github-only
```

The command stores a new immutable version manifest and rebuilds the catalog.
Existing plugin-id/version files cannot be replaced with different content.

CI verifies source repository/tag/commit provenance, immutable GitHub Release
asset URLs, SHA-256 declarations, rename graph validity, and deterministic
catalog generation.
"#,
        request.name, request.description
    )
}

fn marketplace_validate_workflow() -> String {
    format!(
        r#"name: Marketplace validation

on:
  pull_request:
  push:
    branches: [main, master]

permissions:
  contents: read

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@6c977a6ca4077a0ceb28ffbe03f59d46e9ac8772 # v1
        with:
          toolchain: 1.97.0
      - name: Restore Agena plugin developer tool
        id: plugin_dev_cache
        uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          path: .agena-tools
          key: agena-plugin-dev-${{{{ runner.os }}}}-{ref}
      - name: Install pinned Agena plugin developer tool
        if: steps.plugin_dev_cache.outputs.cache-hit != 'true'
        run: >-
          cargo install --locked --root "$GITHUB_WORKSPACE/.agena-tools"
          --git https://github.com/canxin121/agena
          --rev {ref}
          agena-plugin-dev
      - name: Rebuild GitHub-only marketplace
        run: >-
          "$GITHUB_WORKSPACE/.agena-tools/bin/agena-plugin" marketplace build releases
          --output /tmp/agena-marketplace.json
          --github-only
      - name: Verify generated index is committed
        run: cmp /tmp/agena-marketplace.json agena-marketplace.json
      - name: Validate committed index
        run: >-
          "$GITHUB_WORKSPACE/.agena-tools/bin/agena-plugin" marketplace validate
          agena-marketplace.json --github-only
      - name: Reject mutation of published release records
        if: github.event_name == 'pull_request'
        env:
          BASE_SHA: ${{{{ github.event.pull_request.base.sha }}}}
        run: |
          set -euo pipefail
          git fetch --no-tags --depth=1 origin "$BASE_SHA"
          changed="$(git diff --name-status "$BASE_SHA" HEAD -- releases || true)"
          printf '%s\n' "$changed"
          if printf '%s\n' "$changed" | grep -Ev '^A[[:space:]]' | grep -q .; then
            echo "Published release records are immutable; only new release files may be added." >&2
            exit 1
          fi
      - name: Audit GitHub tag and Release provenance
        env:
          GH_TOKEN: ${{{{ github.token }}}}
        run: |
          set -euo pipefail
          jq -r '.plugins[].versions[].source | [.repository, .tag, .commit] | @tsv' agena-marketplace.json \
            | sort -u \
            | while IFS=$'\t' read -r repository tag expected; do
                repo="${{repository#https://github.com/}}"
                object="$(gh api "repos/$repo/git/ref/tags/$tag")"
                type="$(jq -r '.object.type' <<<"$object")"
                actual="$(jq -r '.object.sha' <<<"$object")"
                if [[ "$type" == "tag" ]]; then
                  actual="$(gh api "repos/$repo/git/tags/$actual" --jq '.object.sha')"
                fi
                if [[ "$actual" != "$expected" ]]; then
                  echo "$repo tag $tag resolves to $actual, expected $expected" >&2
                  exit 1
                fi
                gh api "repos/$repo/releases/tags/$tag" >/dev/null
              done
"#,
        ref = AGENA_TEMPLATE_BASELINE_REF,
    )
}

const RUST_TOOLCHAIN: &str = r#"[toolchain]
channel = "1.97.0"
components = ["clippy", "rustfmt"]
profile = "minimal"
"#;

const MARKETPLACE_CONTRIBUTING: &str = r#"# Contributing plugin releases

`agena-marketplace.json` is generated. Do not hand-edit plugin/version records.

1. Publish the plugin from its own GitHub repository.
2. Add its `agena-plugin-release.json` with `agena-plugin marketplace add`.
3. Never modify an existing `releases/<plugin-id>/<version>.json`; publish a new
   plugin version instead.
4. Rebuild with `agena-plugin marketplace build releases --output agena-marketplace.json --github-only`.
5. Open a focused pull request.

Reviewers inspect source provenance, requested capabilities, dependencies,
licensing, security posture, and stable plugin identity.
"#;

const MARKETPLACE_SECURITY: &str = r#"# Security

Marketplace inclusion is not a substitute for reviewing a plugin's repository
and requested Agena capabilities. Accepted public releases are pinned to a
GitHub tag, exact 40-character source commit SHA, immutable GitHub Release asset
URL, and SHA-256 artifact digest.

Use GitHub private security advisories for marketplace or plugin supply-chain
vulnerabilities instead of public issues.
"#;

const MARKETPLACE_PULL_REQUEST_TEMPLATE: &str = r#"## Plugin release

- Plugin id:
- Version:
- Repository:

## Review checklist

- [ ] This adds a new immutable version manifest.
- [ ] Source repository/tag/commit provenance is present.
- [ ] Artifact URLs point to the same repository's immutable GitHub Release.
- [ ] Requested capabilities and dependencies are appropriate and documented.
- [ ] The deterministic GitHub-only marketplace rebuild is clean.
"#;

const CONTRIBUTING: &str = r#"# Contributing

1. Create a focused branch.
2. Run `cargo fmt --all -- --check`.
3. Run `cargo clippy --all-targets --all-features -- -D warnings`.
4. Run `cargo test --all-features` and `agena-plugin validate .`.
5. Keep settings, operations, tools, permissions, and services in the typed Agena SDK contracts.
"#;

const SECURITY: &str = r#"# Security

Do not report vulnerabilities in public issues. Use GitHub's private security advisory flow for this repository.

Release assets are immutable GitHub Release artifacts. Agena verifies their SHA-256 digest before installation; maintainers may additionally configure Ed25519 signatures and trusted keys.
"#;

const PULL_REQUEST_TEMPLATE: &str = r#"## Summary

## Plugin contract changes

- [ ] Settings contract
- [ ] Tools or operations
- [ ] Permissions or services
- [ ] Release packaging

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --all-features`
- [ ] `agena-plugin validate .`
"#;

const BUG_ISSUE_TEMPLATE: &str = r#"name: Bug report
description: Report a reproducible plugin problem
title: "[Bug]: "
labels: [bug]
body:
  - type: textarea
    id: reproduction
    attributes:
      label: Reproduction
      description: Include Agena version, plugin version, platform, logs, and exact steps.
    validations:
      required: true
  - type: textarea
    id: expected
    attributes:
      label: Expected behavior
    validations:
      required: true
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn project_manifest(kind: PluginKind) -> PluginProjectManifest {
        PluginProjectManifest {
            schema_version: 1,
            plugin: PluginProjectMetadata {
                id: "example.hello".to_string(),
                name: "Hello".to_string(),
                description: "Example".to_string(),
                version: "0.1.0".to_string(),
                kind,
                homepage: None,
                repository: Some("https://github.com/example/hello".to_string()),
                license: Some("Apache-2.0".to_string()),
                category: Some("development".to_string()),
                tags: vec!["example".to_string()],
                min_agena_version: Some(">=0.1.0".to_string()),
                args: Vec::new(),
                env: BTreeMap::new(),
                settings: serde_json::json!({}),
                dependencies: Vec::new(),
            },
            release: PluginProjectRelease {
                entrypoint: Some("hello".to_string()),
                include: Vec::new(),
            },
        }
    }

    fn github_source(version: &str) -> PluginReleaseSource {
        PluginReleaseSource {
            repository: "https://github.com/example/hello".to_string(),
            tag: format!("v{version}"),
            commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
            workflow_run_url: Some("https://github.com/example/hello/actions/runs/123".to_string()),
        }
    }

    #[test]
    fn package_and_assemble_produce_installable_release_and_marketplace() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        let fragments = directory.path().join("fragments");
        let release_dir = directory.path().join("release");
        fs::create_dir_all(&project).unwrap();
        let manifest_path = project.join(AGENA_PROJECT_MANIFEST_FILENAME);
        fs::write(
            &manifest_path,
            toml::to_string_pretty(&project_manifest(PluginKind::Stdio)).unwrap(),
        )
        .unwrap();
        let artifact = project.join("hello");
        fs::write(&artifact, b"hello-plugin").unwrap();
        let packaged = package_plugin(PackagePluginRequest {
            manifest_path,
            artifact_path: artifact,
            target: crate::installer::current_target_triple().to_string(),
            output_dir: fragments.clone(),
        })
        .unwrap();
        assert!(packaged.archive_path.is_file());
        assert_eq!(packaged.sha256.len(), 64);
        fs::create_dir_all(&release_dir).unwrap();
        fs::copy(
            &packaged.archive_path,
            release_dir.join(packaged.archive_path.file_name().unwrap()),
        )
        .unwrap();
        let release_base = format!("file://{}", release_dir.display());
        let assembled = assemble_release(AssembleReleaseRequest {
            fragments_dir: fragments,
            output_dir: release_dir.clone(),
            base_url: release_base,
            expected_version: Some("v0.1.0".to_string()),
            source: None,
        })
        .unwrap();
        assert_eq!(assembled.artifact_count, 1);
        let release: PluginReleaseManifest = serde_json::from_slice(
            &fs::read(release_dir.join(AGENA_RELEASE_MANIFEST_FILENAME)).unwrap(),
        )
        .unwrap();
        assert!(
            release.artifacts[0]
                .url
                .as_deref()
                .unwrap()
                .ends_with(packaged.archive_path.file_name().unwrap().to_str().unwrap())
        );
        let marketplace_root = directory.path().join("marketplace");
        let releases_dir = marketplace_root.join("releases");
        let marketplace = marketplace_root.join(crate::manifest::AGENA_MARKETPLACE_FILENAME);
        let added = add_marketplace_release(AddMarketplaceReleaseRequest {
            release_path: release_dir.join(AGENA_RELEASE_MANIFEST_FILENAME),
            releases_dir,
            project_path: None,
            index_path: marketplace.clone(),
            github_only: false,
        })
        .unwrap();
        assert!(!added.already_present);
        let index: RegistryIndex = serde_json::from_slice(&fs::read(marketplace).unwrap()).unwrap();
        index.validate().unwrap();
        assert_eq!(index.plugins[0].id, "example.hello");

        let config_path = directory.path().join("config.json");
        fs::write(&config_path, b"{}\n").unwrap();
        let client = crate::MarketplaceClient::new(
            crate::MarketplaceCache::new(directory.path().join("cache")),
            BTreeMap::new(),
        );
        let installed = client
            .install(crate::InstallRequest {
                registry: crate::RegistrySpec {
                    id: "local-release".to_string(),
                    url: format!(
                        "file://{}",
                        release_dir.join(AGENA_RELEASE_MANIFEST_FILENAME).display()
                    ),
                    require_signature: false,
                    require_github_distribution: false,
                },
                plugin_id: "example.hello".to_string(),
                version: None,
                config_path: config_path.clone(),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: true,
            })
            .unwrap();
        assert_eq!(installed.plugin_id, "example.hello");
        assert!(installed.artifact_path.is_file());
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
        assert_eq!(
            config.pointer("/plugins/list/example.hello/package/kind"),
            Some(&serde_json::json!("stdio"))
        );
        assert_eq!(
            config.pointer("/plugins/list/example.hello/package/command"),
            Some(&serde_json::json!(
                installed.artifact_path.display().to_string()
            ))
        );
    }

    #[test]
    fn assemble_rejects_tag_drift_and_modified_assets() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        let fragments = directory.path().join("fragments");
        fs::create_dir_all(&project).unwrap();
        let manifest_path = project.join(AGENA_PROJECT_MANIFEST_FILENAME);
        fs::write(
            &manifest_path,
            toml::to_string_pretty(&project_manifest(PluginKind::Stdio)).unwrap(),
        )
        .unwrap();
        let artifact = project.join("hello");
        fs::write(&artifact, b"hello-plugin").unwrap();
        let packaged = package_plugin(PackagePluginRequest {
            manifest_path,
            artifact_path: artifact,
            target: crate::installer::current_target_triple().to_string(),
            output_dir: fragments.clone(),
        })
        .unwrap();

        let version_error = assemble_release(AssembleReleaseRequest {
            fragments_dir: fragments.clone(),
            output_dir: directory.path().join("release-version"),
            base_url: "https://github.com/example/hello/releases/download/v0.2.0".to_string(),
            expected_version: Some("v0.2.0".to_string()),
            source: None,
        })
        .unwrap_err();
        assert!(
            version_error
                .to_string()
                .contains("does not match plugin version")
        );

        let github_release_dir = directory.path().join("release-github");
        let strict = assemble_release(AssembleReleaseRequest {
            fragments_dir: fragments.clone(),
            output_dir: github_release_dir.clone(),
            base_url: "https://github.com/example/hello/releases/download/v0.1.0".to_string(),
            expected_version: Some("v0.1.0".to_string()),
            source: Some(github_source("0.1.0")),
        })
        .unwrap();
        let strict_manifest: PluginReleaseManifest =
            serde_json::from_slice(&fs::read(strict.release_manifest_path).unwrap()).unwrap();
        strict_manifest.validate_github_distribution().unwrap();
        assert_eq!(
            strict_manifest.source.as_ref().unwrap().commit,
            "0123456789abcdef0123456789abcdef01234567"
        );

        fs::write(&packaged.archive_path, b"tampered").unwrap();
        let sha_error = assemble_release(AssembleReleaseRequest {
            fragments_dir: fragments,
            output_dir: directory.path().join("release-sha"),
            base_url: "https://github.com/example/hello/releases/download/v0.1.0".to_string(),
            expected_version: Some("v0.1.0".to_string()),
            source: None,
        })
        .unwrap_err();
        assert!(matches!(sha_error, MarketplaceError::Sha256Mismatch { .. }));
    }

    #[test]
    fn scaffold_contains_ci_release_and_closed_project_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("plugin");
        scaffold_plugin(ScaffoldPluginRequest {
            destination: destination.clone(),
            plugin_id: "example.hello".to_string(),
            crate_name: "agena-plugin-hello".to_string(),
            display_name: "Hello Plugin".to_string(),
            description: "Example plugin".to_string(),
            author: "Agena".to_string(),
            repository: Some("https://github.com/example/hello".to_string()),
            kind: PluginTemplateKind::Stdio,
            force: false,
        })
        .unwrap();
        assert!(destination.join(".github/workflows/release.yml").is_file());
        assert_eq!(
            fs::read_to_string(destination.join(".github/CODEOWNERS")).unwrap(),
            "* @example\n"
        );
        assert!(destination.join("src/main.rs").is_file());
        PluginProjectManifest::load(&destination.join(AGENA_PROJECT_MANIFEST_FILENAME)).unwrap();
        let cargo = fs::read_to_string(destination.join("Cargo.toml")).unwrap();
        let ci = fs::read_to_string(destination.join(".github/workflows/ci.yml")).unwrap();
        let release =
            fs::read_to_string(destination.join(".github/workflows/release.yml")).unwrap();
        assert!(cargo.contains(&format!("rev = \"{AGENA_TEMPLATE_BASELINE_REF}\"")));
        assert!(ci.contains(&format!("plugin-ci.yml@{AGENA_TEMPLATE_BASELINE_REF}")));
        assert!(ci.contains(&format!("agena_ref: \"{AGENA_TEMPLATE_BASELINE_REF}\"")));
        assert!(release.contains(&format!("plugin-release.yml@{AGENA_TEMPLATE_BASELINE_REF}")));
        assert!(release.contains(&format!("agena_ref: \"{AGENA_TEMPLATE_BASELINE_REF}\"")));
    }

    #[test]
    fn marketplace_scaffold_and_add_are_deterministic_and_immutable() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("marketplace");
        scaffold_marketplace(ScaffoldMarketplaceRequest {
            destination: destination.clone(),
            name: "Example Marketplace".to_string(),
            description: "Example catalog".to_string(),
            repository: "https://github.com/example/catalog".to_string(),
            owner_name: "Example".to_string(),
            force: false,
        })
        .unwrap();
        let mut project =
            MarketplaceProjectManifest::load(&destination.join(AGENA_MARKETPLACE_PROJECT_FILENAME))
                .unwrap();
        assert_eq!(project.marketplace.name, "Example Marketplace");
        let workflow_path = destination.join(".github/workflows/validate.yml");
        assert!(workflow_path.is_file());
        assert_eq!(
            fs::read_to_string(destination.join(".github/CODEOWNERS")).unwrap(),
            "* @example\n"
        );
        let workflow = fs::read_to_string(&workflow_path).unwrap();
        assert!(workflow.contains(&format!("--rev {AGENA_TEMPLATE_BASELINE_REF}")));
        assert!(workflow.contains("Published release records are immutable"));
        assert!(workflow.contains("gh api \"repos/$repo/git/ref/tags/$tag\""));
        assert!(workflow.contains("github.event.pull_request.base.sha"));
        assert!(workflow.contains("github.token"));
        project.plugins.insert(
            "example.hello".to_string(),
            MarketplacePluginPolicy {
                review_tier: MarketplaceReviewTier::Verified,
                featured: true,
            },
        );
        write_text(
            &destination.join(AGENA_MARKETPLACE_PROJECT_FILENAME),
            toml::to_string_pretty(&project).unwrap().as_str(),
        )
        .unwrap();

        let mut release = project_manifest(PluginKind::Stdio).release_manifest(
            PluginReleaseArtifact {
                target: crate::installer::current_target_triple().to_string(),
                kind: PluginKind::Stdio,
                asset: "example-hello-v0.1.0.tar.gz".to_string(),
                url: Some(
                    "https://github.com/example/hello/releases/download/v0.1.0/example-hello-v0.1.0.tar.gz"
                        .to_string(),
                ),
                sha256: "a".repeat(64),
                signature: None,
                archive: Some(ArchiveSpec::TarGz {
                    entrypoint: "hello".to_string(),
                }),
                command: None,
                args: Vec::new(),
                env: BTreeMap::new(),
                settings: serde_json::json!({}),
            },
        );
        release.source = Some(github_source("0.1.0"));
        let release_path = directory.path().join("release.json");
        write_json(&release_path, &release).unwrap();
        let request = AddMarketplaceReleaseRequest {
            release_path: release_path.clone(),
            releases_dir: destination.join("releases"),
            project_path: Some(destination.join(AGENA_MARKETPLACE_PROJECT_FILENAME)),
            index_path: destination.join(crate::manifest::AGENA_MARKETPLACE_FILENAME),
            github_only: true,
        };
        let first = add_marketplace_release(request.clone()).unwrap();
        assert!(!first.already_present);
        let first_index = fs::read(&first.index_path).unwrap();
        let indexed: RegistryIndex = serde_json::from_slice(&first_index).unwrap();
        assert_eq!(
            indexed.plugins[0].review_tier,
            MarketplaceReviewTier::Verified
        );
        assert!(indexed.plugins[0].featured);
        let second = add_marketplace_release(request.clone()).unwrap();
        assert!(second.already_present);
        assert_eq!(first_index, fs::read(&second.index_path).unwrap());

        release.description = "mutated published metadata".to_string();
        write_json(&release_path, &release).unwrap();
        let error = add_marketplace_release(request).unwrap_err();
        assert!(error.to_string().contains("is immutable"));
    }

    #[test]
    fn marketplace_build_is_deterministic_and_enforces_github_release_policy() {
        let directory = tempfile::tempdir().unwrap();
        let releases = directory.path().join("plugins");
        fs::create_dir_all(&releases).unwrap();
        let mut release = project_manifest(PluginKind::Stdio).release_manifest(
            PluginReleaseArtifact {
                target: crate::installer::current_target_triple().to_string(),
                kind: PluginKind::Stdio,
                asset: "example-hello-v0.1.0.tar.gz".to_string(),
                url: Some(
                    "https://github.com/example/hello/releases/download/v0.1.0/example-hello-v0.1.0.tar.gz"
                        .to_string(),
                ),
                sha256: "a".repeat(64),
                signature: None,
                archive: Some(ArchiveSpec::TarGz {
                    entrypoint: "hello".to_string(),
                }),
                command: None,
                args: Vec::new(),
                env: BTreeMap::new(),
                settings: serde_json::json!({}),
            },
        );
        release.repository = Some("https://github.com/example/hello".to_string());
        release.source = Some(github_source("0.1.0"));
        write_json(&releases.join("example.hello-v0.1.0.json"), &release).unwrap();
        let output = directory
            .path()
            .join(crate::manifest::AGENA_MARKETPLACE_FILENAME);
        let first = build_marketplace(BuildMarketplaceRequest {
            releases_dir: releases.clone(),
            project_path: None,
            output_path: output.clone(),
            github_only: true,
        })
        .unwrap();
        let first_bytes = fs::read(&output).unwrap();
        let second = build_marketplace(BuildMarketplaceRequest {
            releases_dir: releases,
            project_path: None,
            output_path: output.clone(),
            github_only: true,
        })
        .unwrap();
        assert_eq!(first.plugin_count, 1);
        assert_eq!(first.release_count, 1);
        assert_eq!(second.plugin_count, 1);
        assert_eq!(first_bytes, fs::read(output).unwrap());
    }
}
