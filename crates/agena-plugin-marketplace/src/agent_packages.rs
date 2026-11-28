//! Install agent plugin packages from their catalogs and npm distributions.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use semver::Version;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256, Sha384, Sha512};
use tempfile::TempDir;
use url::Url;

use crate::{HttpFetcher, MarketplaceError, installer::extract_tar_gz, write_secure_file};
pub use agena_plugin_contracts::{AgentPluginEcosystem as AgentEcosystem, AgentPluginSource};

mod dependencies;
mod github;
mod npm_dependencies;
mod search;
pub use dependencies::{ClaudeDependencyPlan, ClaudeDependencyVersion, InstalledClaudePackage};
pub use search::{AgentCatalogue, AgentCatalogueEntry};

const MAX_MANIFEST: u64 = 8 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 16_384;

fn marketplace_manifests(ecosystem: AgentEcosystem) -> &'static [&'static str] {
    match ecosystem {
        AgentEcosystem::Codex => &[
            ".agents/plugins/marketplace.json",
            ".codex-plugin/marketplace.json",
        ],
        AgentEcosystem::Claude => &[".claude-plugin/marketplace.json"],
        AgentEcosystem::DeepSeek => &[],
    }
}

fn default_marketplace(ecosystem: AgentEcosystem) -> Option<(&'static str, &'static str)> {
    match ecosystem {
        AgentEcosystem::Codex => Some(("openai-curated", "openai/plugins")),
        AgentEcosystem::Claude => Some((
            "claude-plugins-official",
            "anthropics/claude-plugins-official",
        )),
        AgentEcosystem::DeepSeek => None,
    }
}

pub struct AgentPackageRequest<'a> {
    pub ecosystem: AgentEcosystem,
    pub selector: &'a str,
    pub marketplace: Option<&'a str>,
    pub package_source: Option<&'a str>,
    pub cwd: &'a Path,
    pub accept_command: Option<&'a str>,
}

pub type InstallCommand<'a> =
    dyn Fn(&str, &Path, Duration) -> Result<PathBuf, MarketplaceError> + 'a;

pub struct PreparedAgentPackage {
    pub identity: String,
    pub path: PathBuf,
    pub source: AgentPluginSource,
    pub diagnostics: Vec<String>,
    temp: TempDir,
    origin: PathBuf,
    catalog: Option<(PathBuf, String)>,
    catalog_snapshot: Option<Catalog>,
}

impl PreparedAgentPackage {
    pub fn destination(&self) -> Result<PathBuf, MarketplaceError> {
        Ok(self.origin.join(tree_hash(&self.path)?))
    }

    /// Publish a complete immutable package snapshot. A retained config path
    /// remains valid while a newer version is fetched or another process installs.
    pub fn publish(self) -> Result<PublishedAgentPackage, MarketplaceError> {
        let path = self.destination()?;
        agena_storage_layout::secure_directory(&self.origin)?;
        match fs::rename(&self.path, &path) {
            Ok(()) => {}
            Err(error) if path.is_dir() && !path.is_symlink() => {
                if tree_hash(&path)? != path.file_name().unwrap_or_default().to_string_lossy() {
                    return Err(error.into());
                }
            }
            Err(error) => return Err(error.into()),
        }
        drop(self.temp);
        Ok(PublishedAgentPackage {
            path,
            catalog: self.catalog,
        })
    }
}

pub struct PublishedAgentPackage {
    pub path: PathBuf,
    catalog: Option<(PathBuf, String)>,
}

impl PublishedAgentPackage {
    pub fn save_catalog(&self) -> Result<(), MarketplaceError> {
        if let Some((path, source)) = &self.catalog {
            write_secure_file(
                path,
                &serde_json::to_vec(source)
                    .map_err(|error| MarketplaceError::index_error(&error))?,
            )?;
        }
        Ok(())
    }
}

pub struct AgentPackageInstaller<'a> {
    pub root: &'a Path,
    pub fetcher: &'a dyn HttpFetcher,
    pub command: &'a InstallCommand<'a>,
}

impl AgentPackageInstaller<'_> {
    pub fn prepare(
        &self,
        request: AgentPackageRequest<'_>,
    ) -> Result<PreparedAgentPackage, MarketplaceError> {
        agena_storage_layout::secure_directory(self.root)?;
        let temp = tempfile::Builder::new()
            .prefix(".install-")
            .tempdir_in(self.root)?;
        let source_work = temp.path().join("source");
        fs::create_dir(&source_work)?;
        let path = temp.path().join("package");
        let mut catalog_snapshot = None;
        let mut diagnostics = Vec::new();
        let (identity, origin, catalog, source) = match request.ecosystem {
            AgentEcosystem::DeepSeek => {
                let (name, version) = npm_selector(request.selector)?;
                let root = if let Some(source) = request.package_source {
                    self.package_location(request.cwd, source, &source_work)?
                } else {
                    self.npm(name, version, request.marketplace, &source_work)?
                };
                let mut budget = PackageCopyBudget::default();
                copy_package_with_budget(&root, &path, &mut budget)?;
                let manifest = read_object(&path.join("package.json"))?;
                if manifest.get("name").and_then(Value::as_str) != Some(name) {
                    return Err(invalid(format!("npm package name does not match {name:?}")));
                }
                if manifest
                    .get("dsh")
                    .and_then(|dsh| dsh.pointer("/bundle/patch"))
                    .is_some()
                {
                    diagnostics = npm_dependencies::materialize(
                        self,
                        &path,
                        &manifest,
                        request.marketplace,
                        temp.path(),
                        budget,
                    );
                }
                let package_source = request
                    .package_source
                    .map(|value| normalize_location(request.cwd, value));
                (
                    name.to_owned(),
                    format!(
                        "{}\n{}\n{}",
                        name,
                        request.marketplace.unwrap_or_default(),
                        package_source.as_deref().unwrap_or_default()
                    ),
                    None,
                    AgentPluginSource {
                        ecosystem: request.ecosystem,
                        selector: request.selector.to_owned(),
                        registry: request.marketplace.map(str::to_owned),
                        package_source,
                    },
                )
            }
            ecosystem if request.package_source.is_some() => {
                let (name, market) = split_selector(request.selector)?;
                if market.is_some()
                    || request.marketplace.is_some()
                    || request.accept_command.is_some()
                {
                    return Err(invalid(
                        "use a bare plugin name with a direct package source",
                    ));
                }
                let source = request.package_source.expect("direct package source");
                let resolved = self.package_location(request.cwd, source, &source_work)?;
                copy_package(&resolved, &path)?;
                materialize_manifest(ecosystem, &path, &Map::new(), self.fetcher)?;
                let stored_source =
                    normalize_location(request.cwd, source.strip_prefix("file:").unwrap_or(source));
                (
                    name.to_owned(),
                    format!("{name}\n\n{stored_source}"),
                    None,
                    AgentPluginSource {
                        ecosystem,
                        selector: name.to_owned(),
                        registry: None,
                        package_source: Some(stored_source),
                    },
                )
            }
            ecosystem => {
                let (plugin, market) = split_selector(request.selector)?;
                let market = if request.marketplace.is_none() {
                    market.or_else(|| default_marketplace(ecosystem).map(|(name, _)| name))
                } else {
                    market
                };
                let source = if let Some(source) = request.marketplace {
                    normalize_location(request.cwd, source)
                } else {
                    let market = market
                        .ok_or_else(|| invalid("use plugin@marketplace or supply --marketplace"))?;
                    self.catalog_source(ecosystem, market)?
                };
                let catalog = self.load_catalog(ecosystem, &source, &source_work)?;
                if let Some(market) = market
                    && market != catalog.name
                {
                    return Err(invalid(format!(
                        "marketplace is named {:?}, expected {market:?}",
                        catalog.name
                    )));
                }
                let entry = catalog.entry(plugin)?;
                let identity = format!("{plugin}@{}", catalog.name);
                let resolved = self.entry_root(
                    &catalog,
                    entry,
                    &identity,
                    request.accept_command,
                    &source_work,
                )?;
                copy_package(&resolved, &path)?;
                materialize_manifest(ecosystem, &path, entry, self.fetcher)?;
                let registration = self.catalog_file(ecosystem, &catalog.name);
                catalog_snapshot = Some(catalog);
                (
                    identity.clone(),
                    format!("{identity}\n{source}"),
                    Some((registration, source.clone())),
                    AgentPluginSource {
                        ecosystem,
                        selector: identity,
                        registry: Some(source),
                        package_source: None,
                    },
                )
            }
        };
        let key = hex::encode(Sha256::digest(origin.as_bytes()));
        let origin = self
            .root
            .join("packages")
            .join(request.ecosystem.as_str())
            .join(key);
        Ok(PreparedAgentPackage {
            identity,
            path,
            source,
            diagnostics,
            temp,
            origin,
            catalog,
            catalog_snapshot,
        })
    }

    fn catalog_file(&self, ecosystem: AgentEcosystem, name: &str) -> PathBuf {
        self.root
            .join("catalogs")
            .join(ecosystem.as_str())
            .join(format!("{name}.json"))
    }

    fn catalog_source(
        &self,
        ecosystem: AgentEcosystem,
        name: &str,
    ) -> Result<String, MarketplaceError> {
        match fs::read(self.catalog_file(ecosystem, name)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| MarketplaceError::index_error(&error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => default_marketplace(ecosystem)
                .filter(|(market, _)| *market == name)
                .map(|(_, source)| source.to_owned())
                .ok_or_else(|| invalid(format!(
                    "unknown marketplace {name:?}; pass --marketplace with its directory, Git repository, or catalog URL"
                ))),
            Err(error) => Err(error.into()),
        }
    }

    fn load_catalog(
        &self,
        ecosystem: AgentEcosystem,
        source: &str,
        work: &Path,
    ) -> Result<Catalog, MarketplaceError> {
        let location = Path::new(source);
        let (files, bytes) = if location.is_file() {
            let location = location.canonicalize()?;
            let parent = location
                .parent()
                .ok_or_else(|| invalid("catalog has no parent directory"))?;
            let root = parent
                .file_name()
                .is_some_and(|name| {
                    matches!(name.to_str(), Some(".claude-plugin" | ".codex-plugin"))
                })
                .then(|| parent.parent())
                .flatten()
                .or_else(|| {
                    parent
                        .ends_with(".agents/plugins")
                        .then(|| parent.ancestors().nth(2))
                        .flatten()
                })
                .unwrap_or(parent)
                .to_owned();
            (
                CatalogFiles::Directory(root),
                read_limited(&location, MAX_MANIFEST)?,
            )
        } else if remote_json(source) {
            (
                CatalogFiles::Manifest,
                self.fetcher.fetch(http_url(source)?.as_str())?,
            )
        } else if location.is_dir() {
            read_catalog_directory(ecosystem, location.canonicalize()?)?
        } else if default_marketplace(ecosystem).is_some_and(|(_, repository)| source == repository)
        {
            let snapshot =
                github::Snapshot::default_branch(self.fetcher, source).and_then(|snapshot| {
                    let bytes =
                        snapshot.manifest(self.fetcher, marketplace_manifests(ecosystem))?;
                    Ok((CatalogFiles::GitHub(snapshot), bytes))
                });
            snapshot_or_checkout(snapshot, || {
                read_catalog_directory(
                    ecosystem,
                    checkout(source, None, None, &work.join("marketplace"))?,
                )
            })?
        } else {
            read_catalog_directory(
                ecosystem,
                checkout(source, None, None, &work.join("marketplace"))?,
            )?
        };
        let document = decode_manifest(&bytes)?;
        let name = document
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| valid_segment(name))
            .ok_or_else(|| invalid("marketplace needs a valid name"))?
            .to_owned();
        let mut revision = Sha256::new();
        revision.update(source.as_bytes());
        revision.update([0]);
        revision.update(&bytes);
        if let Some(commit) = files.commit() {
            revision.update(commit.as_bytes());
        }
        let revision = hex::encode(revision.finalize());
        Ok(Catalog {
            name,
            source: source.to_owned(),
            files,
            revision,
            document,
        })
    }

    fn entry_root(
        &self,
        catalog: &Catalog,
        entry: &Map<String, Value>,
        identity: &str,
        approval: Option<&str>,
        work: &Path,
    ) -> Result<PathBuf, MarketplaceError> {
        let source = entry
            .get("source")
            .ok_or_else(|| invalid(format!("{identity} has no package source")))?;
        let fields = |key| source.get(key).and_then(Value::as_str);
        let field =
            |key| fields(key).ok_or_else(|| invalid(format!("{identity} source needs {key}")));
        match source
            .as_str()
            .map(|_| "local")
            .or_else(|| fields("source"))
        {
            Some("local") => {
                let raw = source.as_str().map(Ok).unwrap_or_else(|| field("path"))?;
                let prefix = catalog.plugin_root_for(raw);
                match &catalog.files {
                    CatalogFiles::Directory(root) => catalog_package_directory(root, prefix, raw),
                    CatalogFiles::Manifest => Err(invalid(
                        "a URL-only catalog cannot resolve a local plugin path",
                    )),
                    CatalogFiles::GitHub(snapshot) => {
                        let path = Path::new(raw);
                        if !safe_path(path)
                            || prefix.is_some_and(|prefix| !safe_path(Path::new(prefix)))
                        {
                            return Err(invalid("invalid relative package path"));
                        }
                        let relative = prefix
                            .map_or_else(|| path.to_owned(), |prefix| Path::new(prefix).join(path));
                        let package = snapshot.stage_package(
                            self.fetcher,
                            &relative,
                            &work.join("marketplace-files"),
                        );
                        snapshot_or_checkout(package, || {
                            let root = checkout(
                                &catalog.source,
                                None,
                                Some(&snapshot.commit.to_string()),
                                &work.join("marketplace"),
                            )?;
                            catalog_package_directory(&root, prefix, raw)
                        })
                    }
                }
            }
            Some("github" | "url" | "git-subdir") => {
                let location = if fields("source") == Some("github") {
                    field("repo")?
                } else {
                    field("url")?
                };
                self.checkout_package(location, fields("ref"), fields("sha"), fields("path"), work)
            }
            Some("npm") => {
                let package = field("package")?;
                if package.starts_with("https://") {
                    return self.archive(package, None, work);
                }
                let (name, version) = npm_selector(package)?;
                self.npm(
                    name,
                    version.or_else(|| fields("version")),
                    fields("registry"),
                    work,
                )
            }
            Some("archive") => self.archive(field("url")?, fields("sha256"), work),
            Some("command") => {
                let command = field("command")?;
                if command.is_empty()
                    || command.len() > 500
                    || !command.bytes().all(|byte| (32..127).contains(&byte))
                {
                    return Err(invalid("invalid marketplace install command"));
                }
                let mut hash = Sha256::new();
                for part in [identity, &catalog.revision, command] {
                    hash.update(part.as_bytes());
                    hash.update([0]);
                }
                let digest = hex::encode(hash.finalize());
                if !approval.is_some_and(|value| digest.eq_ignore_ascii_case(value)) {
                    return Err(invalid(format!(
                        "Review the marketplace install command for {identity}:\n{command}\nRetry with --accept-command {digest} after reviewing it."
                    )));
                }
                let timeout = source.get("timeout").and_then(Value::as_u64).unwrap_or(60);
                if !(1..=600).contains(&timeout) {
                    return Err(invalid(
                        "installation command timeout must be 1–600 seconds",
                    ));
                }
                let checkout_root;
                let cwd = match &catalog.files {
                    CatalogFiles::Directory(root) => root.as_path(),
                    CatalogFiles::Manifest => work,
                    CatalogFiles::GitHub(snapshot) => {
                        checkout_root = checkout(
                            &catalog.source,
                            None,
                            Some(&snapshot.commit.to_string()),
                            &work.join("marketplace"),
                        )?;
                        &checkout_root
                    }
                };
                let path = (self.command)(command, cwd, Duration::from_secs(timeout))?;
                if !path.is_absolute() || !path.is_dir() {
                    return Err(invalid(
                        "installation command must print an absolute package directory",
                    ));
                }
                path.canonicalize().map_err(Into::into)
            }
            _ => Err(invalid(format!(
                "unsupported marketplace package source for {identity}"
            ))),
        }
    }

    fn package_location(
        &self,
        cwd: &Path,
        source: &str,
        work: &Path,
    ) -> Result<PathBuf, MarketplaceError> {
        let source = normalize_location(cwd, source.strip_prefix("file:").unwrap_or(source));
        let path = Path::new(&source);
        if path.is_dir() {
            path.canonicalize().map_err(Into::into)
        } else if path.is_file() {
            let bytes = read_limited(path, MAX_PACKAGE_BYTES)?;
            unpack_package(&bytes, &work.join("archive"))
        } else if Url::parse(&source).is_ok_and(|url| {
            url.scheme() == "https"
                && [".tgz", ".tar.gz", ".zip"]
                    .iter()
                    .any(|suffix| url.path().ends_with(suffix))
        }) {
            self.archive(&source, None, work)
        } else {
            let source = source.strip_prefix("git+").unwrap_or(&source);
            let (source, subdir) = source
                .split_once("&path:/")
                .map_or((source, None), |(git, path)| (git, Some(path)));
            self.checkout_package(source, None, None, subdir, work)
        }
    }

    fn checkout_package(
        &self,
        location: &str,
        reference: Option<&str>,
        sha: Option<&str>,
        subdir: Option<&str>,
        work: &Path,
    ) -> Result<PathBuf, MarketplaceError> {
        let git_checkout = || {
            let root = checkout(location, reference, sha, &work.join("repository"))?;
            match subdir {
                Some(path) => directory_under(&root, path),
                None => Ok(root),
            }
        };
        let Some(relative) = subdir.map(Path::new) else {
            return git_checkout();
        };
        if !safe_path(relative) {
            return Err(invalid("invalid relative package path"));
        }
        if !relative
            .components()
            .any(|part| matches!(part, Component::Normal(_)))
        {
            return git_checkout();
        }
        let (url, url_ref) = git_location(location)?;
        let pinned = sha
            .or(reference.or(url_ref))
            .and_then(|value| github::oid(value).ok());
        if let (Some(repository), Some(commit)) = (github::https_repository(&url), pinned) {
            let package = github::Snapshot::at_commit(self.fetcher, &repository, commit).and_then(
                |snapshot| {
                    snapshot.stage_package(self.fetcher, relative, &work.join("repository-files"))
                },
            );
            return snapshot_or_checkout(package, git_checkout);
        }
        git_checkout()
    }

    fn archive(
        &self,
        source: &str,
        digest: Option<&str>,
        work: &Path,
    ) -> Result<PathBuf, MarketplaceError> {
        let url = http_url(source)?;
        let bytes = self.fetcher.fetch(url.as_str())?;
        if let Some(expected) = digest {
            let actual = hex::encode(Sha256::digest(&bytes));
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(invalid(format!(
                    "archive SHA-256 mismatch: expected {expected}, got {actual}"
                )));
            }
        }
        unpack_package(&bytes, &work.join("archive"))
    }

    fn npm(
        &self,
        name: &str,
        request: Option<&str>,
        registry: Option<&str>,
        work: &Path,
    ) -> Result<PathBuf, MarketplaceError> {
        let mut url = http_url(registry.unwrap_or("https://registry.npmjs.org/"))?;
        url.path_segments_mut()
            .map_err(|_| invalid("invalid npm registry URL"))?
            .pop_if_empty()
            .push(name);
        let metadata: Value = serde_json::from_slice(&self.fetcher.fetch(url.as_str())?)
            .map_err(|error| MarketplaceError::index_error(&error))?;
        let (selected, version) = npm_version(&metadata, request.unwrap_or("latest"))?;
        let dist = version
            .get("dist")
            .ok_or_else(|| invalid("npm version is missing dist metadata"))?;
        let tarball = dist
            .get("tarball")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("npm version is missing a tarball URL"))?;
        let bytes = self.fetcher.fetch(http_url(tarball)?.as_str())?;
        let integrity = dist
            .get("integrity")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("npm distribution is missing its integrity digest"))?;
        if !valid_integrity(integrity, &bytes) {
            return Err(invalid(format!("npm integrity mismatch for {name}")));
        }
        let package = unpack_package(&bytes, &work.join("npm"))?;
        let manifest = read_object(&package.join("package.json"))?;
        if manifest.get("name").and_then(Value::as_str) != Some(name)
            || manifest.get("version").and_then(Value::as_str) != Some(selected)
        {
            return Err(invalid(format!(
                "npm archive does not match {name}@{selected}"
            )));
        }
        Ok(package)
    }
}

/// Recover the source for caches created before install selectors were recorded.
pub fn recover_install_source(
    cache: &Path,
    snapshot: &Path,
    package_name: &str,
) -> Result<Option<AgentPluginSource>, MarketplaceError> {
    let Ok(relative) = snapshot.strip_prefix(cache.join("packages")) else {
        return Ok(None);
    };
    let parts: Vec<_> = relative.components().map(|part| part.as_os_str()).collect();
    let [ecosystem, origin, _] = parts.as_slice() else {
        return Ok(None);
    };
    let Some((ecosystem, _)) =
        AgentEcosystem::parse_selector(&format!("{}:", ecosystem.to_string_lossy()))
    else {
        return Ok(None);
    };
    let matches = |key: &str| *origin == hex::encode(Sha256::digest(key.as_bytes())).as_str();
    if ecosystem == AgentEcosystem::DeepSeek {
        return Ok(
            matches(&format!("{package_name}\n\n")).then(|| AgentPluginSource {
                ecosystem,
                selector: package_name.to_owned(),
                registry: None,
                package_source: None,
            }),
        );
    }
    let catalogues = cache.join("catalogs").join(ecosystem.as_str());
    let entries = match fs::read_dir(catalogues) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.extension().is_none_or(|extension| extension != "json") || !valid_segment(name) {
            continue;
        }
        let Ok(registry) = serde_json::from_slice::<String>(&read_limited(&path, MAX_MANIFEST)?)
        else {
            continue;
        };
        let selector = format!("{package_name}@{name}");
        if matches(&format!("{selector}\n{registry}")) {
            return Ok(Some(AgentPluginSource {
                ecosystem,
                selector,
                registry: Some(registry),
                package_source: None,
            }));
        }
    }
    Ok(None)
}

pub fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

pub fn split_selector(selector: &str) -> Result<(&str, Option<&str>), MarketplaceError> {
    let (name, market) = selector
        .split_once('@')
        .map_or((selector, None), |(name, market)| (name, Some(market)));
    if !valid_segment(name) || market.is_some_and(|market| !valid_segment(market)) {
        return Err(invalid("expected plugin or plugin@marketplace"));
    }
    Ok((name, market))
}

pub fn npm_selector(selector: &str) -> Result<(&str, Option<&str>), MarketplaceError> {
    let start = usize::from(selector.starts_with('@'));
    let (name, version) = selector
        .get(start..)
        .and_then(|suffix| suffix.find('@'))
        .map_or((selector, None), |index| {
            (
                &selector[..start + index],
                Some(&selector[start + index + 1..]),
            )
        });
    let valid = if let Some(name) = name.strip_prefix('@') {
        name.split_once('/')
            .is_some_and(|(scope, name)| valid_segment(scope) && valid_segment(name))
    } else {
        valid_segment(name)
    };
    if !valid
        || version.is_some_and(|version| {
            version.trim().is_empty() || version.chars().any(char::is_control)
        })
    {
        return Err(invalid(
            "expected an npm package name with an optional version or dist-tag",
        ));
    }
    Ok((name, version))
}

/// Split an installed dependency name and version selector, including npm aliases.
pub fn npm_dependency_request<'a>(
    name: &'a str,
    request: &'a str,
) -> Result<(&'a str, Option<&'a str>), MarketplaceError> {
    let (_, version) = npm_selector(name)?;
    let request = request.trim();
    if version.is_some()
        || request.is_empty()
        || request.len() > 256
        || request.chars().any(char::is_control)
    {
        return Err(invalid("invalid npm dependency declaration"));
    }
    if let Some(alias) = request.strip_prefix("npm:") {
        return npm_selector(alias);
    }
    Ok((name, Some(request)))
}

/// Match an already bundled version. A dist-tag has no local version table.
pub fn npm_dependency_version_matches(installed: &str, request: Option<&str>) -> bool {
    Version::parse(installed).is_ok()
        && request.is_none_or(|request| {
            let range = request.strip_prefix("workspace:").unwrap_or(request);
            if matches!(request, "workspace:*" | "workspace:^" | "workspace:~") {
                return true;
            }
            ClaudeDependencyVersion::parse(range)
                .map(|range| range.matches(installed))
                .unwrap_or_else(|_| valid_segment(range) || request.starts_with("file:"))
        })
}

struct Catalog {
    name: String,
    source: String,
    files: CatalogFiles,
    revision: String,
    document: Value,
}

enum CatalogFiles {
    Directory(PathBuf),
    GitHub(github::Snapshot),
    Manifest,
}

impl CatalogFiles {
    fn commit(&self) -> Option<git2::Oid> {
        match self {
            Self::GitHub(snapshot) => Some(snapshot.commit),
            Self::Directory(root) => {
                let repo = git2::Repository::open(root).ok()?;
                repo.head().ok()?.target()
            }
            Self::Manifest => None,
        }
    }
}

impl Catalog {
    fn plugin_root_for(&self, path: &str) -> Option<&str> {
        self.document
            .pointer("/metadata/pluginRoot")
            .and_then(Value::as_str)
            .filter(|_| !path.starts_with("./") && !path.contains('/'))
    }

    fn entry(&self, name: &str) -> Result<&Map<String, Value>, MarketplaceError> {
        let plugins = self
            .document
            .get("plugins")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("marketplace plugins must be an array"))?;
        let mut matching = plugins
            .iter()
            .filter_map(Value::as_object)
            .filter(|entry| entry.get("name").and_then(Value::as_str) == Some(name));
        let entry = matching
            .next()
            .ok_or_else(|| MarketplaceError::PluginNotFound(name.to_owned()))?;
        if matching.next().is_some() {
            return Err(invalid(format!("{name:?} is listed multiple times")));
        }
        if entry
            .get("policy")
            .and_then(|policy| policy.get("installation"))
            .and_then(Value::as_str)
            == Some("NOT_AVAILABLE")
        {
            return Err(invalid(format!(
                "{name:?} is unavailable in this marketplace"
            )));
        }
        Ok(entry)
    }
}

fn read_catalog_directory(
    ecosystem: AgentEcosystem,
    root: PathBuf,
) -> Result<(CatalogFiles, Vec<u8>), MarketplaceError> {
    let manifest = marketplace_manifests(ecosystem)
        .iter()
        .map(|path| root.join(path))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            invalid(format!(
                "{} marketplace.json is missing",
                ecosystem.as_str()
            ))
        })?;
    if !manifest.canonicalize()?.starts_with(&root) {
        return Err(invalid("marketplace.json escapes its repository"));
    }
    Ok((
        CatalogFiles::Directory(root),
        read_limited(&manifest, MAX_MANIFEST)?,
    ))
}

fn catalog_package_directory(
    root: &Path,
    prefix: Option<&str>,
    path: &str,
) -> Result<PathBuf, MarketplaceError> {
    match prefix {
        Some(prefix) => directory_under(&directory_under(root, prefix)?, path),
        None => directory_under(root, path),
    }
}

fn snapshot_or_checkout<T>(
    snapshot: Result<T, MarketplaceError>,
    checkout: impl FnOnce() -> Result<T, MarketplaceError>,
) -> Result<T, MarketplaceError> {
    snapshot.or_else(|snapshot_error| {
        checkout().map_err(|error| invalid(format!(
            "GitHub snapshot failed ({snapshot_error}). Git repository fallback failed ({error})."
        )))
    })
}

fn materialize_manifest(
    ecosystem: AgentEcosystem,
    root: &Path,
    entry: &Map<String, Value>,
    fetcher: &dyn HttpFetcher,
) -> Result<(), MarketplaceError> {
    let manifest = match ecosystem {
        AgentEcosystem::Codex => root.join(".codex-plugin/plugin.json"),
        AgentEcosystem::Claude => root.join(".claude-plugin/plugin.json"),
        _ => return Ok(()),
    };
    let candidate = root.join("plugin.json");
    if ecosystem == AgentEcosystem::Codex {
        if manifest.is_file() || candidate.is_file() {
            return Ok(());
        }
        if root.join(".claude-plugin/plugin.json").is_file() {
            return materialize_manifest(AgentEcosystem::Claude, root, entry, fetcher);
        }
    }
    let existing = if manifest.is_file() {
        Some(read_object(&manifest)?)
    } else if ecosystem == AgentEcosystem::Claude && candidate.is_file() {
        Some(read_object(&candidate)?)
    } else {
        None
    };
    let component_fields = [
        "skills",
        "agents",
        "commands",
        "hooks",
        "outputStyles",
        "themes",
    ];
    if existing.is_some()
        && ecosystem == AgentEcosystem::Claude
        && entry.get("strict") == Some(&Value::Bool(false))
        && component_fields
            .iter()
            .any(|field| entry.contains_key(*field))
    {
        return Err(invalid(
            "Claude marketplace entry has conflicting component declarations (strict: false)",
        ));
    }
    let mut document = existing.unwrap_or_default();
    for (key, value) in entry {
        if matches!(key.as_str(), "source" | "strict" | "policy") {
            continue;
        }
        if matches!(key.as_str(), "defaultEnabled" | "displayName") {
            document.insert(key.clone(), value.clone());
            continue;
        }
        if ecosystem == AgentEcosystem::Claude && component_fields.contains(&key.as_str()) {
            merge_component(&mut document, key, value);
        } else {
            document.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    if ecosystem == AgentEcosystem::Claude
        && let Some(servers) = document.get_mut("mcpServers")
    {
        let sources = match servers {
            Value::Array(sources) => sources.as_mut_slice(),
            source => std::slice::from_mut(source),
        };
        for source in sources {
            let Some(url) = source.as_str().and_then(|source| Url::parse(source).ok()) else {
                continue;
            };
            let Some(extension) = ["mcpb", "dxt"].into_iter().find(|extension| {
                url.path()
                    .rsplit_once('.')
                    .is_some_and(|(_, ending)| ending == *extension)
            }) else {
                continue;
            };
            let bytes = fetcher.fetch(http_url(url.as_str())?.as_str())?;
            if !bytes.starts_with(b"PK") || bytes.len() as u64 > MAX_PACKAGE_BYTES {
                return Err(invalid("remote MCP bundle must be a bounded ZIP archive"));
            }
            let digest = hex::encode(Sha256::digest(&bytes));
            let relative = format!(".agena-mcp-bundles/{digest}.{extension}");
            write_secure_file(&root.join(&relative), &bytes)?;
            *source = Value::String(format!("./{relative}"));
        }
    }
    write_secure_file(
        &manifest,
        &serde_json::to_vec_pretty(&document)
            .map_err(|error| MarketplaceError::index_error(&error))?,
    )?;
    Ok(())
}

fn merge_component(document: &mut Map<String, Value>, key: &str, value: &Value) {
    let Some(current) = document.get_mut(key) else {
        document.insert(key.to_owned(), value.clone());
        return;
    };
    if key == "hooks" || key == "commands" {
        if let (Some(current), Some(extra)) = (current.as_object_mut(), value.as_object()) {
            current.extend(extra.clone());
            return;
        }
    }
    let mut sources = current
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![current.clone()]);
    if let Some(values) = value.as_array() {
        sources.extend(values.iter().cloned());
    } else {
        sources.push(value.clone());
    }
    *current = Value::Array(sources);
}

fn normalize_location(cwd: &Path, value: &str) -> String {
    let path = cwd.join(value);
    path.canonicalize().map_or_else(
        |_| value.to_owned(),
        |path| path.to_string_lossy().into_owned(),
    )
}

fn remote_json(value: &str) -> bool {
    Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.path().ends_with(".json"))
}

fn http_url(value: &str) -> Result<Url, MarketplaceError> {
    let url = Url::parse(value).map_err(|error| MarketplaceError::index_error(&error))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid(
            "expected an HTTPS URL or a local HTTP registry URL",
        ));
    }
    Ok(url)
}

fn git_location(value: &str) -> Result<(String, Option<&str>), MarketplaceError> {
    let (value, reference) = value
        .strip_prefix("github:")
        .unwrap_or(value)
        .split_once('#')
        .map_or(
            (value.strip_prefix("github:").unwrap_or(value), None),
            |(value, reference)| (value, Some(reference)),
        );
    if let Ok(url) = Url::parse(value)
        && matches!(url.scheme(), "https" | "ssh" | "file")
    {
        if url.password().is_some() {
            return Err(invalid("Git source cannot contain a password"));
        }
        return Ok((value.to_owned(), reference));
    }
    if value.starts_with("git@") && value.split_once(':').is_some() {
        return Ok((value.to_owned(), reference));
    }
    let (repo, short_ref) = value
        .rsplit_once('@')
        .map_or((value, None), |(repo, reference)| (repo, Some(reference)));
    if repo
        .split_once('/')
        .is_some_and(|(owner, repo)| valid_segment(owner) && valid_segment(repo))
    {
        return Ok((
            format!("https://github.com/{repo}"),
            reference.or(short_ref),
        ));
    }
    Err(invalid(format!("invalid Git package source {value:?}")))
}

fn checkout(
    location: &str,
    reference: Option<&str>,
    sha: Option<&str>,
    dest: &Path,
) -> Result<PathBuf, MarketplaceError> {
    let (location, url_ref) = git_location(location)?;
    let git_error = |error: git2::Error| invalid(format!("Git checkout: {error}"));
    let repo = git2::Repository::init(dest).map_err(git_error)?;
    let mut remote = repo.remote_anonymous(&location).map_err(git_error)?;
    let mut options = git2::FetchOptions::new();
    options.proxy_options(git_proxy_options());
    if !location.starts_with("file://") {
        options.depth(1);
    }
    const PACKAGE_REF: &str = "refs/agena/package";
    let mut fetch = |revision: &str| {
        let refspec = format!("+{revision}:{PACKAGE_REF}");
        remote.fetch(&[refspec], Some(&mut options), None)?;
        // libgit2 can return success for an unmatched refspec. Verify that
        // the requested ref was actually fetched before trying a tag.
        repo.find_reference(PACKAGE_REF)?.peel_to_commit()
    };
    let commit = if let Some(sha) = sha {
        if sha.len() != 40 {
            return Err(invalid("Git sha must be a full commit ID"));
        }
        git2::Oid::from_str(sha).map_err(git_error)?;
        fetch(sha)
    } else if let Some(reference) = reference.or(url_ref) {
        if reference.starts_with("refs/")
            || (reference.len() == 40 && reference.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            fetch(reference)
        } else {
            fetch(&format!("refs/heads/{reference}"))
                .or_else(|_| fetch(&format!("refs/tags/{reference}")))
        }
    } else {
        fetch("HEAD")
    }
    .map_err(git_error)?;
    repo.set_head_detached(commit.id()).map_err(git_error)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .map_err(git_error)?;
    dest.canonicalize().map_err(Into::into)
}

fn git_proxy_options() -> git2::ProxyOptions<'static> {
    let mut proxy = git2::ProxyOptions::new();
    proxy.auto();
    proxy
}

fn npm_version<'a>(
    metadata: &'a Value,
    request: &str,
) -> Result<(&'a str, &'a Value), MarketplaceError> {
    let versions = metadata
        .get("versions")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("npm metadata is missing versions"))?;
    let exact = metadata
        .get("dist-tags")
        .and_then(|tags| tags.get(request))
        .and_then(Value::as_str)
        .unwrap_or(request);
    let exact = exact.trim_start_matches('v');
    if let Some((version, manifest)) = versions.get_key_value(exact) {
        return Ok((version, manifest));
    }
    if Version::parse(exact).is_ok() {
        return Err(invalid(format!("npm version {exact:?} is unavailable")));
    }
    let requirements = ClaudeDependencyVersion::parse(request)
        .map_err(|_| invalid(format!("invalid npm version range or dist-tag {request:?}")))?;
    versions
        .iter()
        .filter_map(|(version, data)| {
            let candidate = Version::parse(version).ok()?;
            requirements
                .matches(version)
                .then_some((candidate, version, data))
        })
        .max_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, version, value)| (version.as_str(), value))
        .ok_or_else(|| invalid(format!("no npm version matches {request:?}")))
}

fn valid_integrity(integrity: &str, bytes: &[u8]) -> bool {
    let digest = Sha512::digest(bytes);
    if integrity
        .split_whitespace()
        .any(|value| value.strip_prefix("sha512-").is_some())
    {
        return integrity.split_whitespace().any(|value| {
            value
                .strip_prefix("sha512-")
                .is_some_and(|value| value == STANDARD.encode(digest))
        });
    }
    integrity
        .split_whitespace()
        .any(|value| match value.split_once('-') {
            Some(("sha384", value)) => value == STANDARD.encode(Sha384::digest(bytes)),
            Some(("sha256", value)) => value == STANDARD.encode(Sha256::digest(bytes)),
            _ => false,
        })
}

fn unpack_package(bytes: &[u8], dest: &Path) -> Result<PathBuf, MarketplaceError> {
    fs::create_dir(dest)?;
    if bytes.starts_with(b"PK") {
        extract_package_zip(bytes, dest)?;
    } else {
        extract_tar_gz(bytes, dest).map_err(invalid)?;
    }
    if ![
        "plugin.json",
        ".codex-plugin",
        ".claude-plugin",
        "package.json",
        "SKILL.md",
        "skills",
        "hooks",
    ]
    .iter()
    .any(|path| dest.join(path).exists())
    {
        let mut entries = fs::read_dir(dest)?.filter_map(Result::ok);
        if let Some(entry) = entries.next()
            && entry.path().is_dir()
            && entries.next().is_none()
        {
            return Ok(entry.path());
        }
    }
    Ok(dest.to_owned())
}

/// Extract a bounded ZIP into a newly staged package directory.
pub fn extract_package_zip(bytes: &[u8], dest: &Path) -> Result<(), MarketplaceError> {
    let archive_error = |error: zip::result::ZipError| invalid(format!("package zip: {error}"));
    let mut zip = zip::ZipArchive::new(io::Cursor::new(bytes)).map_err(archive_error)?;
    if zip.len() > MAX_ENTRIES {
        return Err(invalid("package zip has too many entries"));
    }
    let mut unpacked = 0;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(archive_error)?;
        let name = file
            .enclosed_name()
            .filter(|path| safe_path(path))
            .ok_or_else(|| invalid("unsafe path in package zip"))?;
        if file.is_symlink() {
            return Err(invalid("package zip contains a symbolic link"));
        }
        let path = dest.join(name);
        if file.is_dir() {
            fs::create_dir_all(path)?;
            continue;
        }
        unpacked += file.size();
        if unpacked > MAX_PACKAGE_BYTES {
            return Err(invalid("expanded package zip is too large"));
        }
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| invalid("invalid archive target"))?,
        )?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        let size = file.size();
        if io::copy(&mut file.by_ref().take(size + 1), &mut output)? != size {
            return Err(invalid(
                "package zip entry size did not match its directory record",
            ));
        }
        set_mode(&path, file.unix_mode().unwrap_or_default())?;
    }
    Ok(())
}

fn safe_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|part| match part {
            Component::CurDir => true,
            Component::Normal(name) => !name.to_string_lossy().contains(['\\', ':']),
            _ => false,
        })
}

fn directory_under(root: &Path, relative: &str) -> Result<PathBuf, MarketplaceError> {
    let relative = Path::new(relative);
    if !safe_path(relative) {
        return Err(invalid("invalid relative package path"));
    }
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(root) || !path.is_dir() {
        return Err(invalid("package path escapes its source repository"));
    }
    Ok(path)
}

fn package_entries(root: &Path) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> {
    walkdir::WalkDir::new(root)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
}

fn copy_package(root: &Path, dest: &Path) -> Result<(), MarketplaceError> {
    copy_package_with_budget(root, dest, &mut PackageCopyBudget::default())
}

#[derive(Default)]
struct PackageCopyBudget {
    entries: usize,
    bytes: u64,
}

fn copy_package_with_budget(
    root: &Path,
    dest: &Path,
    budget: &mut PackageCopyBudget,
) -> Result<(), MarketplaceError> {
    if dest.starts_with(root) {
        return Err(invalid("the package source contains its staging directory"));
    }
    fs::create_dir(dest)?;
    for entry in package_entries(root) {
        let entry = entry.map_err(|error| invalid(error.to_string()))?;
        if budget.entries >= MAX_ENTRIES {
            return Err(invalid("package has too many files"));
        }
        budget.entries += 1;
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| MarketplaceError::index_error(&error))?;
        if !safe_path(relative) {
            return Err(invalid("unsafe path in the package directory"));
        }
        let destination = dest.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&destination)?;
        } else if entry.file_type().is_file() {
            let mut input = File::open(entry.path())?;
            let metadata = input.metadata()?;
            let remaining = MAX_PACKAGE_BYTES.saturating_sub(budget.bytes);
            if metadata.len() > remaining {
                return Err(invalid("package is too large"));
            }
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&destination)?;
            budget.bytes += io::copy(&mut input.by_ref().take(remaining + 1), &mut output)?;
            if budget.bytes > MAX_PACKAGE_BYTES {
                return Err(invalid("package is too large"));
            }
            set_mode(&destination, file_mode(&metadata))?;
        } else {
            return Err(invalid(format!(
                "package contains a link or special file at {}",
                relative.display()
            )));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111
}
#[cfg(not(unix))]
fn file_mode(_: &fs::Metadata) -> u32 {
    0
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600 | (mode & 0o111)))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn tree_hash(root: &Path) -> Result<String, MarketplaceError> {
    let mut digest = Sha256::new();
    let canonical_root = root.canonicalize()?;
    for entry in package_entries(root) {
        let entry = entry.map_err(|error| invalid(error.to_string()))?;
        let path = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| MarketplaceError::index_error(&error))?;
        digest.update(path.to_string_lossy().as_bytes());
        digest.update([0]);
        if entry.file_type().is_file() {
            let mut file = File::open(entry.path())?;
            let metadata = file.metadata()?;
            digest.update(metadata.len().to_le_bytes());
            digest.update(file_mode(&metadata).to_le_bytes());
            let mut buffer = [0_u8; 32 * 1024];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
        } else if entry.file_type().is_symlink() {
            // npm and Bun create relative executable links under node_modules.
            // A snapshot must remain self-contained after its staging move.
            let target = fs::read_link(entry.path())?;
            if !path.starts_with("node_modules")
                || target.is_absolute()
                || !entry.path().canonicalize()?.starts_with(&canonical_root)
            {
                return Err(invalid(format!(
                    "unsafe package link at {}",
                    path.display()
                )));
            }
            digest.update([1]);
            digest.update(target.as_os_str().as_encoded_bytes());
        } else if !entry.file_type().is_dir() {
            return Err(invalid(format!(
                "unsupported package file {}",
                path.display()
            )));
        }
        digest.update([0xff]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn read_object(path: &Path) -> Result<Map<String, Value>, MarketplaceError> {
    serde_json::from_slice(&read_limited(path, MAX_MANIFEST)?)
        .map_err(|error| MarketplaceError::index_error(&error))
}

fn decode_manifest(bytes: &[u8]) -> Result<Value, MarketplaceError> {
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err(invalid("marketplace catalog is too large"));
    }
    serde_json::from_slice(bytes).map_err(|error| MarketplaceError::index_error(&error))
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, MarketplaceError> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("package input needs a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("package file exceeds its size limit"));
    }
    Ok(bytes)
}

fn invalid(message: impl Into<String>) -> MarketplaceError {
    MarketplaceError::Index(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Fixture(BTreeMap<String, Vec<u8>>);

    impl HttpFetcher for Fixture {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError> {
            self.0
                .get(url)
                .cloned()
                .ok_or_else(|| invalid(format!("unexpected fetch {url}")))
        }
    }

    fn command_forbidden(_: &str, _: &Path, _: Duration) -> Result<PathBuf, MarketplaceError> {
        panic!("this test does not execute install scripts")
    }

    #[test]
    fn git_marketplace_install_resolves_ref_and_reuses_registered_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let repository = git2::Repository::init(&source).unwrap();
        fs::create_dir_all(source.join(".agents/plugins")).unwrap();
        fs::create_dir_all(source.join("plugin/skills/review")).unwrap();
        fs::write(source.join(".agents/plugins/marketplace.json"),
            br#"{"name":"team","plugins":[{"name":"review","source":{"source":"local","path":"./plugin"}}]}"#).unwrap();
        fs::write(source.join("plugin/skills/review/SKILL.md"), "# Review").unwrap();
        let mut index = repository.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        let tree = repository.find_tree(index.write_tree().unwrap()).unwrap();
        let author = git2::Signature::now("Fixture", "fixture@example.test").unwrap();
        let revision = repository
            .commit(Some("HEAD"), &author, &author, "fixture", &tree, &[])
            .unwrap();
        repository
            .tag(
                "v1",
                &repository.find_object(revision, None).unwrap(),
                &author,
                "fixture package",
                false,
            )
            .unwrap();
        let location = format!("{}#v1", Url::from_file_path(&source).unwrap());
        let cache = temp.path().join("cache");
        let installer = AgentPackageInstaller {
            root: &cache,
            fetcher: &Fixture(BTreeMap::new()),
            command: &command_forbidden,
        };
        let install = |marketplace| {
            installer.prepare(AgentPackageRequest {
                ecosystem: AgentEcosystem::Codex,
                selector: "review@team",
                marketplace,
                package_source: None,
                cwd: temp.path(),
                accept_command: None,
            })
        };
        let package = install(Some(&location)).unwrap();
        assert_eq!(package.identity, "review@team");
        assert!(package.path.join(".codex-plugin/plugin.json").is_file());
        assert!(package.path.join("skills/review/SKILL.md").is_file());
        let first = package.publish().unwrap();
        first.save_catalog().unwrap();
        let next = install(None).unwrap().publish().unwrap();
        assert_eq!(first.path, next.path);
    }

    fn tarball(manifest: &[u8]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(encoder);
        for (path, bytes) in [
            ("package/package.json", manifest),
            ("package/bundle.yml", b"[]\n".as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            tar.append_data(&mut header, path, bytes).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn npm_install_checks_distribution_integrity_and_unwraps_package() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = tarball(
            br#"{"name":"@team/demo","version":"1.4.0","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
        );
        let registry = "http://127.0.0.1:8123/";
        let tarball = format!("{registry}bundle.tgz");
        let npm_url = format!("{registry}@team%2Fdemo");
        let metadata = serde_json::json!({
            "dist-tags": {"latest": "1.4.0"},
            "versions": {
                "1.4.0": {
                    "dist": {
                        "tarball": tarball,
                        "integrity": format!("sha512-{}", STANDARD.encode(Sha512::digest(&bytes)))
                    }
                }
            }
        });
        let fixture = Fixture(BTreeMap::from([
            (npm_url.clone(), serde_json::to_vec(&metadata).unwrap()),
            (tarball.clone(), bytes),
        ]));
        let request = || AgentPackageRequest {
            ecosystem: AgentEcosystem::DeepSeek,
            selector: "@team/demo@^1.0.0",
            marketplace: Some(registry),
            package_source: None,
            cwd: temp.path(),
            accept_command: None,
        };
        let installer = |fetcher| AgentPackageInstaller {
            root: temp.path(),
            fetcher,
            command: &command_forbidden,
        };
        let result = installer(&fixture)
            .prepare(request())
            .unwrap()
            .publish()
            .unwrap();
        assert_eq!(
            read_object(&result.path.join("package.json")).unwrap()["version"],
            "1.4.0"
        );
        let mut invalid = Fixture(fixture.0.clone());
        invalid.0.insert(tarball, b"different tarball".to_vec());
        assert!(
            installer(&invalid)
                .prepare(request())
                .err()
                .unwrap()
                .to_string()
                .contains("integrity mismatch")
        );
    }

    #[test]
    fn deepseek_npm_snapshot_keeps_aliased_and_transitive_versions() {
        let registry = "http://127.0.0.1:8123/";
        let releases = [
            serde_json::json!({"name":"demo","version":"1.0.0","dsh":{"bundle":{"patch":"bundle.yml"}},
                "dependencies":{"helper":"2.0.0", "provider":"npm:@team/provider@^1.0.0"}}),
            serde_json::json!({"name":"@team/provider","version":"1.1.0","dependencies":{"helper":"1.0.0"}}),
            serde_json::json!({"name":"helper","version":"1.0.0","dependencies":{"provider":"npm:@team/provider@^1.0.0"}}),
            serde_json::json!({"name":"helper","version":"2.0.0"}),
        ];
        let mut fixture = BTreeMap::new();
        let mut indexes = BTreeMap::<String, Map<String, Value>>::new();
        for (index, release) in releases.iter().enumerate() {
            let name = release["name"].as_str().unwrap();
            let version = release["version"].as_str().unwrap();
            let archive = tarball(&serde_json::to_vec(release).unwrap());
            let url = format!("{registry}{index}.tgz");
            indexes.entry(name.replace('/', "%2F")).or_default().insert(version.into(), serde_json::json!({
                "dist": {"tarball": url, "integrity": format!("sha512-{}", STANDARD.encode(Sha512::digest(&archive)))},
            }));
            fixture.insert(url, archive);
        }
        for (name, versions) in indexes {
            fixture.insert(
                format!("{registry}{name}"),
                serde_json::to_vec(&serde_json::json!({"versions": versions})).unwrap(),
            );
        }
        let temp = tempfile::tempdir().unwrap();
        let installer = AgentPackageInstaller {
            root: temp.path(),
            fetcher: &Fixture(fixture),
            command: &command_forbidden,
        };
        let prepared = installer
            .prepare(AgentPackageRequest {
                ecosystem: AgentEcosystem::DeepSeek,
                selector: "demo@1.0.0",
                marketplace: Some(registry),
                package_source: None,
                cwd: temp.path(),
                accept_command: None,
            })
            .unwrap();
        assert!(
            prepared.diagnostics.is_empty(),
            "{:?}",
            prepared.diagnostics
        );
        let installed = prepared.publish().unwrap().path;
        let inspect = |path: &str| read_object(&installed.join(path)).unwrap();
        assert_eq!(
            inspect("node_modules/helper/package.json")["version"],
            "2.0.0"
        );
        assert_eq!(
            inspect("node_modules/provider/package.json")["name"],
            "@team/provider"
        );
        assert_eq!(
            inspect("node_modules/provider/node_modules/helper/package.json")["version"],
            "1.0.0"
        );
        assert!(
            !installed
                .join("node_modules/provider/node_modules/helper/node_modules/provider")
                .exists()
        );
    }
}
